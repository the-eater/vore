use crate::{GlobalConfig, InstanceConfig, QemuCommandBuilder};
use anyhow::{Context, Error};
use beau_collector::BeauCollector;
use qapi::qmp::QMP;
use qapi::Qmp;
use std::fmt;
use std::fmt::{Debug, Formatter};
use std::fs::{read_link, OpenOptions};
use std::io;
use std::io::{BufReader, ErrorKind, Read, Write};
use std::option::Option::Some;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::result::Result::Ok;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

#[derive(Debug)]
pub struct VirtualMachine {
    working_dir: PathBuf,
    config: InstanceConfig,
    global_config: GlobalConfig,
    process: Option<Child>,
    control_socket: Option<ControlSocket>,
}

struct ControlSocket {
    unix_stream: CloneableUnixStream,
    qmp: Qmp<qapi::Stream<BufReader<CloneableUnixStream>, CloneableUnixStream>>,
    info: QMP,
}

impl Debug for ControlSocket {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ControlSocket")
            .field(&self.unix_stream)
            .finish()
    }
}

const AUTO_UNBIND_BLACKLIST: &[&str] = &["nvidia"];

impl VirtualMachine {
    pub fn new(
        config: InstanceConfig,
        global_config: &GlobalConfig,
        working_dir: PathBuf,
    ) -> VirtualMachine {
        VirtualMachine {
            working_dir,
            config,
            global_config: global_config.clone(),
            process: None,
            control_socket: None,
        }
    }

    pub fn prepare(&mut self, execute_fixes: bool, force: bool) -> Result<(), anyhow::Error> {
        let mut results = vec![];
        results.extend(self.prepare_disks());
        results.extend(self.prepare_vfio(execute_fixes, force));
        results
            .into_iter()
            .bcollect::<()>()
            .with_context(|| format!("Failed to prepare VM {}", self.config.name))?;
        Ok(())
    }

    ///
    /// Doesn't really prepare them, but mostly checks if the user has permissions to read them
    ///
    pub fn prepare_disks(&self) -> Vec<Result<(), anyhow::Error>> {
        self.config
            .disks
            .iter()
            .map(|disk| {
                OpenOptions::new()
                    .read(true)
                    .open(&disk.path)
                    .with_context(|| format!("Failed to open disk {}", disk.path))?;

                Ok(())
            })
            .collect::<Vec<_>>()
    }

    /// Prepare VFIO related shenanigans,
    /// This includes if requested via [execute_fixes] unbinding the requested vfio pci devices
    /// And binding them to vfio-pci
    ///
    /// With [execute_fixes] set to false, it will only check if everything is sane, and the correct driver is loaded
    fn prepare_vfio(&mut self, execute_fixes: bool, force: bool) -> Vec<Result<(), Error>> {
        self.config.vfio.iter().map(|vfio| {
            let pci_driver_path = format!("/sys/bus/pci/devices/{:#}/driver", vfio.address);

            let driver = match read_link(&pci_driver_path) {
                Ok(driver_link) => {
                    let driver_path = driver_link.to_str().ok_or_else(|| {
                        anyhow::anyhow!(
                            "Path to device driver for PCI device at {} is not valid utf-8",
                            vfio.address
                        )
                    })?;
                    let driver = driver_path.split("/").last().ok_or_else(|| {
                        anyhow::anyhow!(
                        "Path to device driver for PCI device at {} doesn't have a path to a driver",
                        vfio.address
                    )
                    })?;

                    driver.to_string()
                }

                Err(err) if err.kind() == ErrorKind::NotFound => "".to_string(),

                Err(err) => return Err(err.into()),
            };

            let is_blacklisted = AUTO_UNBIND_BLACKLIST.contains(&driver.as_str()) && !force;

            if driver != "vfio-pci" && (!execute_fixes || is_blacklisted) {
                if !driver.is_empty() && is_blacklisted {
                    anyhow::bail!("PCI device {} it's current driver is {}, but to be used with VFIO needs to be set to vfio-pci, this driver ({1}) has been blacklisted from automatic rebinding because it can't be cleanly unbound, please make sure this device is unbound before running vore", vfio.address, driver)
                } else if !driver.is_empty() {
                    anyhow::bail!("PCI device {} it's current driver is {}, but to be used with VFIO needs to be set to vfio-pci", vfio.address, driver)
                } else {
                    anyhow::bail!("PCI device at {} currently has no driver, but to be used with VFIO needs to be set to vfio-pci", vfio.address)
                }
            }

            if driver != "vfio-pci" && execute_fixes && !is_blacklisted {
                let address = format!("{:#}\n", vfio.address).into_bytes();

                if !driver.is_empty() {
                    let mut unbind = std::fs::OpenOptions::new().append(true).open(format!(
                        "/sys/bus/pci/devices/{:#}/driver/unbind",
                        vfio.address
                    ))?;

                    unbind.write_all(&address)?;
                }

                {
                    let mut driver_override = OpenOptions::new().append(true).open(format!(
                        "/sys/bus/pci/devices/{:#}/driver_override",
                        vfio.address
                    ))?;

                    driver_override.write_all(b"vfio-pci\n")?;
                }

                let mut probe = OpenOptions::new()
                    .append(true)
                    .open("/sys/bus/pci/drivers_probe")?;
                probe.write_all(&address)?;

                let new_link = read_link(&pci_driver_path)?;
                if !new_link.ends_with("vfio-pci") {
                    anyhow::bail!("Tried to bind {} to vfio-pci but failed to do so (see /sys/bus/pci/devices/{:#} for more info)", vfio.address, vfio.address)
                }
            }

            Ok(())
        })
            .collect::<Vec<_>>()
    }

    pub fn get_cmd_line(&self) -> Result<Vec<String>, anyhow::Error> {
        let builder = QemuCommandBuilder::new(&self.global_config, self.working_dir.clone())?;
        builder.build(&self.config)
    }

    pub fn pin_qemu_threads(&self) {
        let pid = if let Some(child) = &self.process {
            child.id()
        } else {
            return;
        };
    }

    pub fn start(&mut self) -> Result<(), anyhow::Error> {
        if let Some(proc) = &mut self.process {
            if proc.try_wait()?.is_none() {
                return Ok(());
            }
        }

        let mut command = Command::new("qemu-system-x86_64");
        command.args(self.get_cmd_line()?);
        self.process = Some(command.spawn()?);

        let mut res = || {
            let qemu_control_socket = format!("{}/qemu.sock", self.working_dir.to_str().unwrap());

            let mut unix_stream = UnixStream::connect(&qemu_control_socket);
            let mut time = 30;
            while let Err(err) = unix_stream {
                if time < 0 {
                    Err(err).context(format!(
                        "After 30 seconds, QEMU Control socket ({}) didn't come up",
                        qemu_control_socket
                    ))?;
                }

                std::thread::sleep(Duration::from_secs(1));
                unix_stream = UnixStream::connect(&qemu_control_socket);
                time -= 1;
            }

            let unix_stream = unix_stream.unwrap();
            let unix_stream = CloneableUnixStream(Arc::new(Mutex::new(unix_stream)));
            let mut qmp = Qmp::from_stream(unix_stream.clone());

            let handshake = qmp.handshake()?;

            let mut control_socket = ControlSocket {
                unix_stream,
                qmp,
                info: handshake,
            };

            self.pin_qemu_threads();

            control_socket
                .qmp
                .execute(&qapi_qmp::cont {})
                .context("Failed to send start command on qemu control socket")?;

            control_socket.qmp.nop()?;

            while let Some(event) = control_socket.qmp.events().next() {
                println!("event: {:?}", event);
            }

            self.control_socket = Some(control_socket);

            Ok(())
        };

        let result_ = res();
        if result_.is_err() {
            if let Some(mut qemu) = self.process.take() {
                qemu.kill()?;
                qemu.wait()?;
            }
        }

        result_
    }
}

#[derive(Clone, Debug)]
struct CloneableUnixStream(Arc<Mutex<UnixStream>>);

impl CloneableUnixStream {
    pub fn lock(&self) -> Result<MutexGuard<'_, UnixStream>, std::io::Error> {
        self.0.lock().map_err(|_| {
            io::Error::new(
                ErrorKind::Other,
                anyhow::anyhow!("Failed to lock UnixStream"),
            )
        })
    }
}

impl Read for CloneableUnixStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let res = self.lock()?.read(buf);
        if let Ok(size) = res {
            println!("READ: {}", String::from_utf8_lossy(&buf[..size]));
        }
        res
    }
}

impl Write for CloneableUnixStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.lock()?.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.lock()?.flush()
    }
}
