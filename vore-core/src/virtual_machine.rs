use crate::{GlobalConfig, InstanceConfig, QemuCommandBuilder};
use anyhow::{Context, Error};
use beau_collector::BeauCollector;
use qapi::qmp::{QMP, Event};
use qapi::{Qmp};
use std::{fmt, mem};
use std::fmt::{Debug, Formatter, Display};
use std::fs::{read_link, OpenOptions, read_dir};
use std::io;
use std::io::{BufReader, ErrorKind, Read, Write};
use std::option::Option::Some;
use std::os::unix::net::UnixStream;
use std::path::{PathBuf, Path};
use std::process::{Child, Command};
use std::result::Result::Ok;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use qapi_qmp::QmpCommand;
use std::str::FromStr;
use libc::{cpu_set_t, CPU_SET, sched_setaffinity};
use crate::cpu_list::CpuList;
use std::os::unix::prelude::AsRawFd;
use serde::{Deserialize, Serialize};

#[derive(Eq, PartialEq, Copy, Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VirtualMachineState {
    Loaded,
    Prepared,
    Stopped,
    Paused,
    Running,
}

impl Display for VirtualMachineState {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            VirtualMachineState::Loaded => write!(f, "loaded"),
            VirtualMachineState::Prepared => write!(f, "prepared"),
            VirtualMachineState::Stopped => write!(f, "stopped"),
            VirtualMachineState::Paused => write!(f, "paused"),
            VirtualMachineState::Running => write!(f, "running")
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct VirtualMachineInfo {
    pub name: String,
    pub working_dir: PathBuf,
    pub config: InstanceConfig,
    pub state: VirtualMachineState,
}

#[derive(Debug)]
pub struct VirtualMachine {
    working_dir: PathBuf,
    state: VirtualMachineState,
    config: InstanceConfig,
    global_config: GlobalConfig,
    process: Option<Child>,
    control_socket: Option<ControlSocket>,
}

struct ControlSocket {
    unix_stream: CloneableUnixStream,
    qmp: Qmp<qapi::Stream<BufReader<CloneableUnixStream>, CloneableUnixStream>>,
    _info: QMP,
}

impl Debug for ControlSocket {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ControlSocket")
            .field(&self.unix_stream)
            .finish()
    }
}

const AUTO_UNBIND_BLACKLIST: &[&str] = &["nvidia", "amdgpu"];

impl VirtualMachine {
    pub fn new<P: AsRef<Path>>(
        config: InstanceConfig,
        global_config: &GlobalConfig,
        working_dir: P,
    ) -> VirtualMachine {
        VirtualMachine {
            working_dir: working_dir.as_ref().to_path_buf(),
            state: VirtualMachineState::Loaded,
            config,
            global_config: global_config.clone(),
            process: None,
            control_socket: None,
        }
    }

    pub fn name(&self) -> &str {
        &self.config.name
    }

    pub fn info(&self) -> VirtualMachineInfo {
        VirtualMachineInfo {
            name: self.name().to_string(),
            working_dir: self.working_dir.clone(),
            config: self.config.clone(),
            state: self.state,
        }
    }

    pub fn prepare(&mut self, execute_fixes: bool, force: bool) -> Result<(), anyhow::Error> {
        let mut results = vec![];
        results.extend(self.prepare_disks());
        results.extend(self.prepare_vfio(execute_fixes, force));
        results.extend(self.prepare_shm());
        results.extend(self.prepare_sockets());
        results
            .into_iter()
            .bcollect::<()>()
            .with_context(|| format!("Failed to prepare VM {}", self.config.name))?;

        if self.state == VirtualMachineState::Loaded {
            self.state = VirtualMachineState::Prepared;
        }
        Ok(())
    }

    pub fn prepare_shm(&mut self) -> Vec<Result<(), anyhow::Error>> {
        let mut shm = vec![];
        if self.config.looking_glass.enabled {
            if self.config.looking_glass.mem_path.is_empty() {
                self.config.looking_glass.mem_path = format!("/dev/shm/vore/{}/looking-glass", self.config.name);
            }

            shm.push(&self.config.looking_glass.mem_path);
        }

        if self.config.scream.enabled {
            if self.config.scream.mem_path.is_empty() {
                self.config.scream.mem_path = format!("/dev/shm/vore/{}/scream", self.config.name);
            }

            shm.push(&self.config.scream.mem_path);
        }

        shm
            .into_iter()
            .map(|x| Path::new(x))
            .filter_map(|x| x.parent())
            .filter(|x| !x.is_dir())
            .map(|x| std::fs::create_dir_all(&x).with_context(|| format!("Failed creating directories for shared memory ({:?})", x)))
            .collect()
    }

    pub fn prepare_sockets(&mut self) -> Vec<Result<(), anyhow::Error>> {
        let mut sockets = vec![];
        if self.config.spice.enabled {
            if self.config.spice.socket_path.is_empty() {
                self.config.spice.socket_path = self.working_dir.join("spice.sock").to_str().unwrap().to_string();
            }

            sockets.push(&self.config.spice.socket_path);
        }

        sockets
            .into_iter()
            .map(|x| Path::new(x))
            .filter_map(|x| x.parent())
            .filter(|x| !x.is_dir())
            .map(|x| std::fs::create_dir_all(&x).with_context(|| format!("Failed creating directories for shared memory ({:?})", x)))
            .collect()
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
    ///
    /// [force] can be given to auto-bind PCI devices that are blacklisted anyway. this can result in vore indefinitely hanging.
    fn prepare_vfio(&mut self, execute_fixes: bool, force: bool) -> Vec<Result<(), Error>> {
        if self.config.vfio.is_empty() {
            return vec![];
        }

        match Command::new("modprobe")
            .arg("vfio-pci")
            .spawn()
            .and_then(|mut x| x.wait())
        {
            Err(err) => return vec![Err(err.into())],
            Ok(x) if !x.success() => {
                return vec![Err(anyhow::anyhow!(
                    "Failed to load vfio-pci kernel module. can't use VFIO"
                ))];
            }
            Ok(_) => {}
        }

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
                    // Unbind the PCI device from the current driver
                    let mut unbind = std::fs::OpenOptions::new().append(true).open(format!(
                        "/sys/bus/pci/devices/{:#}/driver/unbind",
                        vfio.address
                    ))?;

                    unbind.write_all(&address)?;
                }

                {
                    // Set a driver override
                    let mut driver_override = OpenOptions::new().append(true).open(format!(
                        "/sys/bus/pci/devices/{:#}/driver_override",
                        vfio.address
                    ))?;

                    driver_override.write_all(b"vfio-pci\n")?;
                }

                {
                    // Probe the PCI device so the driver override is picked up
                    let mut probe = OpenOptions::new()
                        .append(true)
                        .open("/sys/bus/pci/drivers_probe")?;
                    probe.write_all(&address)?;
                }

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

    pub fn pin_qemu_threads(&self) -> Result<(), anyhow::Error> {
        let pid = if let Some(child) = &self.process {
            child.id()
        } else {
            return Ok(());
        };

        let list = CpuList::adjacent(self.config.cpu.amount as usize);
        if list.is_none() {
            // If we are over provisioning CPU's there's not much use to pinning
            return Ok(());
        }

        let list = list.unwrap();

        let mut kvm_threads = vec![];
        for item in read_dir(format!("/proc/{}/task", pid))? {
            let entry = item?;
            if !entry.file_type()?.is_dir() {
                continue;
            }

            let res = entry.file_name().to_str().ok_or_else(|| anyhow::anyhow!("")).and_then(|x| usize::from_str(x).map_err(From::from));
            if res.is_err() {
                continue;
            }

            let tid = res.unwrap();
            let name = entry.path().join("comm");
            let comm = std::fs::read_to_string(name)?;
            if comm.starts_with("CPU ") {
                let nr = comm.chars().skip(4).take_while(|x| x.is_ascii_digit()).collect::<String>();
                let cpu_id = usize::from_str(&nr).unwrap();
                kvm_threads.push((tid, cpu_id));
            }
        }

        for (tid, cpu_id) in kvm_threads {
            if cpu_id >= list.len() {
                // ???
                continue;
            }

            let cpu = &list[cpu_id];
            unsafe {
                let mut set = mem::zeroed::<cpu_set_t>();
                CPU_SET(cpu.id, &mut set);
                sched_setaffinity(tid as i32, mem::size_of::<cpu_set_t>(), &set);
            }
        }

        Ok(())
    }

    pub fn boop(&mut self) -> Result<(), anyhow::Error> {
        if let Some(qmp) = self.control_socket.as_mut() {
            qmp.qmp.nop()?;
        }

        self.process_qmp_events()
    }

    fn process_qmp_events(&mut self) -> Result<(), anyhow::Error> {
        let events = if let Some(qmp) = self.control_socket.as_mut() {
            // While we could iter, we keep hold of the mutable reference, so it's easier to just collect the events
            qmp.qmp.events().collect::<Vec<_>>()
        } else {
            return Ok(());
        };

        for event in events {
            println!("Event: {:?}", event);

            match event {
                Event::STOP { .. } => {
                    if self.state != VirtualMachineState::Stopped {
                        self.state = VirtualMachineState::Paused;
                    }
                }
                Event::RESUME { .. } => {
                    self.state = VirtualMachineState::Running;
                }
                Event::SHUTDOWN { .. } => {
                    self.state = VirtualMachineState::Stopped;
                }

                _ => {}
            }
        }

        Ok(())
    }

    pub fn pause(&mut self) -> Result<(), anyhow::Error> {
        if self.state != VirtualMachineState::Running {
            return Ok(());
        }

        self.send_qmp_command(&qapi_qmp::stop {})?;

        Ok(())
    }

    fn send_qmp_command<C: QmpCommand>(&mut self, command: &C) -> Result<C::Ok, anyhow::Error> {
        let res = if let Some(qmp) = self.control_socket.as_mut() {
            qmp.qmp.execute(command)?
        } else {
            anyhow::bail!("No control socket available")
        };

        self.process_qmp_events()?;
        Ok(res)
    }

    pub fn stop(&mut self) -> Result<(), anyhow::Error> {
        if self.process.is_none() || self.control_socket.is_none() || self.state == VirtualMachineState::Stopped {
            return Ok(());
        }

        self.send_qmp_command(&qapi_qmp::system_powerdown {})?;
        Ok(())
    }

    pub fn wait_till_stopped(&mut self) -> Result<(), anyhow::Error> {
        self.wait(None, VirtualMachineState::Stopped)?;
        Ok(())
    }

    pub fn quit(&mut self) -> Result<(), anyhow::Error> {
        if self.control_socket.is_none() {
            return Ok(());
        }

        match self.send_qmp_command(&qapi_qmp::quit {}) {
            Err(err) if err.downcast_ref::<io::Error>().map_or(false, |x| x.kind() == io::ErrorKind::UnexpectedEof) => {}
            err => { err?; }
        }

        Ok(())
    }

    fn wait(&mut self, duration: Option<Duration>, target_state: VirtualMachineState) -> Result<bool, anyhow::Error> {
        let start = Instant::now();
        while duration.map_or(true, |dur| (Instant::now() - start) < dur) {
            let has_socket = self.control_socket.as_mut()
                .map(|x| x.qmp.nop())
                .transpose()?
                .is_some();

            if !has_socket {
                return Ok(self.state == target_state);
            }

            self.process_qmp_events()?;

            if self.state == target_state {
                return Ok(true);
            }

            if duration.is_some() {
                std::thread::sleep(Duration::from_millis(500));
            } else {
                std::thread::sleep(Duration::from_secs(5));
            }
        }

        Ok(self.state == target_state)
    }

    pub fn start(&mut self) -> Result<(), anyhow::Error> {
        if let Some(proc) = &mut self.process {
            if proc.try_wait()?.is_none() {
                return Ok(());
            }
        }

        if self.state == VirtualMachineState::Loaded {
            self.prepare(true, false)?
        }

        let mut command = Command::new("qemu-system-x86_64");
        command.args(self.get_cmd_line().context("Failed to generate qemu command line")?);
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

                if let Some(proc) = self.process.as_mut() {
                    if let Some(_) = proc.try_wait()? {
                        anyhow::bail!("QEMU quit early")
                    }
                }

                time -= 1;
            }

            let unix_stream = CloneableUnixStream::new(unix_stream.unwrap());
            let mut qmp = Qmp::from_stream(unix_stream.clone());

            let handshake = qmp.handshake()?;

            let mut control_socket = ControlSocket {
                unix_stream,
                qmp,
                _info: handshake,
            };

            self.pin_qemu_threads()?;

            control_socket
                .qmp
                .execute(&qapi_qmp::cont {})
                .context("Failed to send start command on qemu control socket")?;

            control_socket.qmp.nop()?;
            self.control_socket = Some(control_socket);

            self.process_qmp_events()?;

            Ok(())
        };

        let result_ = res();
        if result_.is_err() {
            if let Some(mut qemu) = self.process.take() {
                let _ = qemu.kill();
                qemu.wait()?;
            }
        }

        result_
    }

    pub fn control_stream(&self) -> Option<&CloneableUnixStream> {
        self.control_socket.as_ref().map(|x| &x.unix_stream)
    }
}

#[derive(Clone, Debug)]
pub struct CloneableUnixStream(Arc<Mutex<UnixStream>>);

impl CloneableUnixStream {
    pub fn new(unix_stream: UnixStream) -> CloneableUnixStream {
        CloneableUnixStream(Arc::new(Mutex::new(unix_stream)))
    }

    pub fn lock(&self) -> Result<MutexGuard<'_, UnixStream>, std::io::Error> {
        self.0.lock().map_err(|_| {
            io::Error::new(
                ErrorKind::Other,
                anyhow::anyhow!("Failed to lock UnixStream"),
            )
        })
    }
}

impl AsRawFd for CloneableUnixStream {
    fn as_raw_fd(&self) -> i32 {
        self.lock().unwrap().as_raw_fd()
    }
}

impl Read for CloneableUnixStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let res = self.lock()?.read(buf);
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