use anyhow::Context;
use polling::{Event, Poller};
use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
use signal_hook::iterator::{Handle, Signals, SignalsInfo};
use signal_hook::low_level::signal_name;
use std::collections::HashMap;
use std::fs;
use std::fs::{read_dir, read_to_string, DirEntry};
use std::io::{Read, Write};
use std::mem::size_of;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{SocketAddr, UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;
use std::{io, mem};
use vore_core::consts::{VORE_CONFIG, VORE_DIRECTORY, VORE_SOCKET};
use vore_core::rpc::{AllRequests, AllResponses, Command, CommandCenter, DiskPreset, Response};
use vore_core::utils::get_username_by_uid;
use vore_core::{rpc, QemuCommandBuilder, VirtualMachineInfo};
use vore_core::{GlobalConfig, InstanceConfig, VirtualMachine};

#[derive(Debug)]
struct RpcConnection {
    stream: UnixStream,
    address: SocketAddr,
    buffer: Vec<u8>,
    uid: u32,
    user: Option<String>,
    pid: i32,
}

impl Write for RpcConnection {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.stream.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

impl Read for RpcConnection {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.stream.read(buf)
    }
}

#[allow(clippy::char_lit_as_u8)]
const NEWLINE: u8 = '\n' as u8;

impl RpcConnection {
    pub fn handle_input(
        &mut self,
        own_id: usize,
    ) -> Result<(bool, Vec<(usize, Command)>), anyhow::Error> {
        let mut still_open = true;
        loop {
            let mut buffer = vec![0u8; 4096];
            match self.stream.read(&mut buffer) {
                Ok(amount) if amount == 0 => {
                    still_open = false;
                    break;
                }
                Ok(amount) => self.buffer.extend_from_slice(&buffer[..amount]),
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
                Err(err) => return Err(err.into()),
            };
        }

        let mut buffer = mem::take(&mut self.buffer);
        if still_open {
            self.buffer = buffer.split_off(
                buffer
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(_, x)| **x == NEWLINE)
                    .map(|(idx, _)| idx + 1)
                    .unwrap_or(buffer.len()),
            );
        }

        let mut commands = vec![];

        for part in buffer.split(|x| *x == NEWLINE) {
            if part.is_empty() {
                continue;
            }

            let lossy = String::from_utf8_lossy(part);

            match CommandCenter::read_command(&lossy) {
                Ok(cmd) => {
                    log::debug!("Got command: {:?}", cmd);
                    commands.push((own_id, cmd));
                }

                Err(err) => {
                    log::info!("RPC Connection produced error: {}", err)
                }
            }
        }

        Ok((still_open, commands))
    }
}

#[derive(Clone, Eq, PartialEq, Debug)]
enum EventTarget {
    RpcListener,
    Machine(String),
    RpcConnection(usize),
    None,
}

#[derive(Debug)]
pub struct Daemon {
    event_key_storage: Vec<EventTarget>,
    global_config: GlobalConfig,
    machines: HashMap<String, VirtualMachine>,
    connections: Vec<Option<RpcConnection>>,
    rpc_listener: UnixListener,
    socket_path: PathBuf,
    poller: Poller,
    signals: SignalsInfo,
    signals_handle: Handle,
    queue: Vec<Event>,
    command_queue: Vec<(usize, Command)>,
}

impl Daemon {
    pub fn new() -> Result<Daemon, anyhow::Error> {
        log::debug!("Loading global config ({})", VORE_CONFIG);
        let toml = std::fs::read_to_string(VORE_CONFIG)?;
        let mut global_config = GlobalConfig::load(&toml)?;
        log::debug!("Creating vore daemon");
        let signals = Signals::new(&[SIGINT, SIGHUP])?;
        let handle = signals.handle();
        log::debug!("Bound signal handlers");
        let poller = Poller::new().context("Failed to make poller")?;
        let socket_path = PathBuf::from_str(VORE_SOCKET)?;
        let rpc_listener =
            UnixListener::bind(&socket_path).context("Failed to bind vore socket")?;

        global_config.vore.chown(socket_path.to_str().unwrap())?;

        rpc_listener.set_nonblocking(true)?;
        log::debug!("Bound to {}", VORE_SOCKET);

        let mut daemon = Daemon {
            event_key_storage: vec![],
            global_config,
            machines: Default::default(),
            connections: vec![],
            rpc_listener,
            poller,
            signals,
            signals_handle: handle,
            queue: vec![],
            command_queue: vec![],
            socket_path,
        };

        daemon.init()?;
        Ok(daemon)
    }

    pub fn init(&mut self) -> Result<(), anyhow::Error> {
        let new_key = self.add_target(EventTarget::RpcListener);
        self.poller
            .add(&self.rpc_listener, Event::readable(new_key))?;

        Ok(())
    }

    pub fn load_definitions(&mut self) -> Result<(), anyhow::Error> {
        let vm_dir = PathBuf::from(format!("{}/definitions", VORE_DIRECTORY));
        if !vm_dir.is_dir() {
            return Ok(());
        }

        let dir_iter =
            read_dir(&vm_dir).with_context(|| format!("Failed to list {:?} for vm's", &vm_dir))?;

        let mut process = |entry: Result<DirEntry, io::Error>| -> anyhow::Result<()> {
            let entry = entry?;
            let file_name = entry.path();
            let path = file_name.to_str().context("Entry has invalid UTF-8 path")?;
            if !path.ends_with(".toml") {
                return Ok(());
            }

            let toml = read_to_string(path)
                .with_context(|| format!("Failed to read VM definition {}", path))?;
            self.load_virtual_machine(&toml, None, false)?;
            Ok(())
        };

        for entry in dir_iter {
            if let Err(err) = process(entry) {
                log::error!("Failed parsing entry in {:?}: {:?}", vm_dir, err);
            }
        }

        Ok(())
    }

    pub fn reserve_vfio_devices(&mut self) {
        for machine in self.machines.values() {
            for vfio_device in machine.vfio_devices() {
                if !vfio_device.reserve {
                    continue;
                }

                if let Err(err) = VirtualMachine::prepare_vfio_device(true, true, &vfio_device) {
                    log::error!(
                        "Failed to reserve PCI device {} for {}: {:?}",
                        vfio_device.address,
                        machine.name(),
                        err
                    );
                } else {
                    log::info!(
                        "Reserved PCI device {} for {}",
                        vfio_device.address,
                        machine.name()
                    );
                }
            }
        }
    }

    pub fn auto_start_machines(&mut self) {
        for machine in self.machines.values_mut() {
            if !machine.should_auto_start() {
                continue;
            }

            if let Err(err) = machine.start() {
                log::error!("Failed to auto-start {}: {:?}", machine.name(), err);
            } else {
                log::info!("Autostarted {}", machine.name());
            }
        }
    }

    pub fn run(&mut self) -> Result<(), anyhow::Error> {
        self.load_definitions()?;
        self.reserve_vfio_devices();
        self.auto_start_machines();

        loop {
            let res = self
                .wait()
                .context("Got error while waiting for new notifications");
            match res {
                // Interrupted is uh "always" when we get a signal
                Err(err)
                    if err
                        .downcast_ref::<io::Error>()
                        .map(|x| x.kind() == io::ErrorKind::Interrupted)
                        .unwrap_or(false) =>
                {
                    if !self.handle_exit_code()? {
                        break;
                    }
                }
                err => err?,
            }

            if !self.handle_event_queue()? {
                break;
            }

            self.handle_command_queue()?;
        }

        // TODO: clean up
        log::info!("vore daemon has ended");
        std::fs::remove_file(&self.socket_path).context("Failed cleaning up socket")?;
        Ok(())
    }

    pub fn handle_command_queue(&mut self) -> Result<(), anyhow::Error> {
        while let Some((id, command)) = self.command_queue.pop() {
            let resp = self.handle_command(&command);
            if let Err(err) = &resp {
                log::warn!("Command {:?} failed with error: {:?}", command, err)
            }

            if let Some(conn) = self.connections[id].as_mut() {
                conn.write_all(CommandCenter::write_answer(&command, resp)?.as_bytes())?;
            }
        }

        Ok(())
    }

    pub fn load_virtual_machine(
        &mut self,
        toml: &str,
        working_directory: Option<String>,
        save: bool,
    ) -> anyhow::Result<VirtualMachineInfo> {
        let config = InstanceConfig::from_toml(&toml)?;
        if save {
            let save_file = format!("{}/definitions/{}.toml", VORE_DIRECTORY, config.name);
            let file_path = Path::new(&save_file);
            if let Some(parent_dir) = file_path.parent() {
                if !parent_dir.is_dir() {
                    fs::create_dir_all(parent_dir)?;
                }
            }

            fs::write(&save_file, toml).with_context(|| {
                format!(
                    "Failed to save vm definition for {} to {}",
                    config.name, save_file
                )
            })?;
        }

        let working_dir = working_directory
            .unwrap_or_else(|| format!("{}/instance/{}", VORE_DIRECTORY, config.name));
        let vm = VirtualMachine::new(config, &self.global_config, working_dir);
        let info = vm.info();
        self.mount_machine(vm);
        Ok(info)
    }

    pub fn handle_command(&mut self, command: &Command) -> Result<AllResponses, anyhow::Error> {
        let resp = match &command.data {
            AllRequests::Info(_) => rpc::InfoResponse {
                name: "vore".to_string(),
                version: format!(
                    "{}.{}.{}{}",
                    env!("CARGO_PKG_VERSION_MAJOR"),
                    env!("CARGO_PKG_VERSION_MINOR"),
                    env!("CARGO_PKG_VERSION_PATCH"),
                    option_env!("CARGO_PKG_VERSION_PRE").unwrap_or("")
                ),
            }
            .into_enum(),
            AllRequests::List(_) => rpc::ListResponse {
                items: self.machines.values().map(|x| x.info()).collect(),
            }
            .into_enum(),
            AllRequests::Load(val) => rpc::LoadResponse {
                info: self.load_virtual_machine(
                    &val.toml,
                    val.working_directory.as_ref().cloned(),
                    val.save,
                )?,
            }
            .into_enum(),
            AllRequests::Prepare(val) => {
                if let Some(machine) = self.machines.get_mut(&val.name) {
                    machine.prepare(true, false)?;
                } else {
                    anyhow::bail!("No machine with the name {} exists", val.name);
                }

                rpc::PrepareResponse {}.into_enum()
            }
            AllRequests::Start(val) => {
                let cloned = if let Some(machine) = self.machines.get_mut(&val.name) {
                    machine.start()?;

                    machine.control_stream().cloned()
                } else {
                    anyhow::bail!("No machine with the name {} exists", val.name);
                };

                if let Some(cloned) = cloned {
                    let new_id = self.add_target(EventTarget::Machine(val.name.clone()));
                    self.poller.add(&cloned, Event::readable(new_id))?;
                }

                rpc::StartResponse {}.into_enum()
            }
            AllRequests::Stop(val) => {
                if let Some(machine) = self.machines.get_mut(&val.name) {
                    machine.stop()?;
                } else {
                    anyhow::bail!("No machine with the name {} exists", val.name);
                }

                rpc::StartResponse {}.into_enum()
            }
            AllRequests::Unload(_) => {
                anyhow::bail!("Unimplemented");
            }
            AllRequests::Kill(val) => {
                if let Some(machine) = self.machines.get_mut(&val.name) {
                    machine.quit()?;
                } else {
                    anyhow::bail!("No machine with the name {} exists", val.name);
                }

                rpc::StartResponse {}.into_enum()
            }
            AllRequests::DiskPresets(_) => {
                let builder =
                    QemuCommandBuilder::new(&self.global_config, PathBuf::from("/dev/empty"))?;

                rpc::DiskPresetsResponse {
                    presets: builder
                        .list_presets()?
                        .into_iter()
                        .map(|(name, description)| DiskPreset { name, description })
                        .collect(),
                }
                .into_enum()
            }
        };

        Ok(resp)
    }

    pub fn handle_exit_code(&mut self) -> Result<bool, anyhow::Error> {
        for signal in self.signals.pending() {
            log::info!(
                "Received signal {} ({})",
                signal_name(signal).unwrap_or("<unknown>"),
                signal
            );
            match signal {
                SIGINT | SIGTERM => return Ok(false),
                _ => {}
            }
        }
        Ok(true)
    }

    pub fn handle_event_queue(&mut self) -> Result<bool, anyhow::Error> {
        let queue = mem::take(&mut self.queue);
        for event in queue {
            let target = self.event_key_storage.get(event.key).cloned();
            if let Some(item) = target {
                log::debug!("Handling {:?} from target {:?}", event, item);

                match item {
                    EventTarget::RpcListener => {
                        self.poller
                            .modify(&self.rpc_listener, Event::readable(event.key))?;
                        self.accept_rpc_connections()?;
                    }
                    EventTarget::Machine(name) if self.machines.contains_key(&name) => {
                        if let Some(machine) = self.machines.get_mut(&name) {
                            machine.boop()?;
                        }

                        if let Some(control_socket) =
                            self.machines.get(&name).and_then(|x| x.control_stream())
                        {
                            self.poller
                                .modify(control_socket, Event::readable(event.key))?;
                        }
                    }
                    EventTarget::RpcConnection(rpc_connection_id)
                        if self
                            .connections
                            .get(rpc_connection_id)
                            .map(Option::is_some)
                            .unwrap_or(false) =>
                    {
                        let (still_open, mut commands) = if let Some(rpc_connection) =
                            &mut self.connections[rpc_connection_id]
                        {
                            let input_res = rpc_connection.handle_input(rpc_connection_id)?;
                            if input_res.0 {
                                self.poller
                                    .modify(&rpc_connection.stream, Event::readable(event.key))?;
                            }

                            input_res
                        } else {
                            (false, vec![])
                        };

                        if !still_open {
                            log::info!("RPC connection {} closed", rpc_connection_id);
                            self.connections[rpc_connection_id] = None;
                        }

                        self.command_queue.append(&mut commands)
                    }
                    _ => continue,
                }
            }
        }

        Ok(true)
    }

    fn accept_rpc_connections(&mut self) -> Result<(), anyhow::Error> {
        loop {
            let (stream, address) = match self.rpc_listener.accept() {
                Ok(value) => value,
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(err) => return Err(err.into()),
            };

            stream.set_nonblocking(true)?;

            let ucred = unsafe {
                let mut ucred: libc::ucred = mem::zeroed();
                let mut length = size_of::<libc::ucred>() as u32;
                libc::getsockopt(
                    stream.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_PEERCRED,
                    (&mut ucred) as *mut _ as _,
                    &mut length,
                );
                ucred
            };

            let user = get_username_by_uid(ucred.uid)?;

            let conn = RpcConnection {
                stream,
                address,
                buffer: vec![],
                uid: ucred.uid,
                user,
                pid: ucred.pid,
            };

            log::info!(
                "Got new RPC connection from {} (pid: {}, socket: {:?})",
                conn.user.as_ref().map_or_else(
                    || format!("uid:{}", conn.uid),
                    |x| format!("{} ({})", x, conn.uid),
                ),
                conn.pid,
                conn.address,
            );

            let id = self.add_rpc_connection(conn);
            let event_target = self.add_target(EventTarget::RpcConnection(id));
            self.poller.add(
                &self.connections[id].as_ref().unwrap().stream,
                Event::readable(event_target),
            )?;
        }
    }

    pub fn wait(&mut self) -> Result<(), anyhow::Error> {
        self.poller
            .wait(&mut self.queue, Some(Duration::from_secs(5)))?;
        Ok(())
    }

    fn add_target(&mut self, event_target: EventTarget) -> usize {
        let id = self
            .event_key_storage
            .iter()
            .enumerate()
            .find(|(_, target)| target.eq(&&EventTarget::None));
        if let Some((id, _)) = id {
            self.event_key_storage[id] = event_target;
            return id;
        }

        let new_id = self.event_key_storage.len();
        self.event_key_storage.push(event_target);
        new_id
    }

    fn add_rpc_connection(&mut self, rpc_connection: RpcConnection) -> usize {
        let id = self
            .connections
            .iter()
            .enumerate()
            .find(|(_, target)| target.is_none());
        if let Some((id, _)) = id {
            self.connections[id] = Some(rpc_connection);
            return id;
        }

        let new_id = self.connections.len();
        self.connections.push(Some(rpc_connection));
        new_id
    }

    fn mount_machine(&mut self, vm: VirtualMachine) {
        log::info!("Loaded {}", vm.name());
        let name = vm.name().to_string();
        self.machines.insert(name, vm);
    }
}
