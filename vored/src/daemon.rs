use std::collections::{HashMap};
use vore_core::{VirtualMachine, InstanceConfig, GlobalConfig, GLOBAL_CONFIG_LOCATION};
use std::os::unix::net::{UnixListener, SocketAddr, UnixStream};
use polling::{Poller, Event};
use std::time::Duration;
use anyhow::Context;
use std::{mem, io};
use std::io::{Read, Write};
use vore_core::rpc::{CommandCenter, Response, Command, AllRequests, AllResponses};
use vore_core::rpc;
use signal_hook::low_level::{signal_name};
use signal_hook::consts::{SIGINT, SIGTERM, SIGHUP};
use std::path::PathBuf;
use std::str::FromStr;
use signal_hook::iterator::{SignalsInfo, Signals, Handle};
use std::os::unix::io::AsRawFd;
use std::ffi::CStr;
use std::mem::size_of;

#[derive(Debug)]
struct RPCConnection {
    stream: UnixStream,
    address: SocketAddr,
    buffer: Vec<u8>,
    uid: u32,
    user: Option<String>,
    pid: i32,
}

impl Write for RPCConnection {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.stream.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

impl Read for RPCConnection {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.stream.read(buf)
    }
}

const NEWLINE: u8 = '\n' as u8;

impl RPCConnection {
    pub fn handle_input(&mut self, own_id: usize) -> Result<(bool, Vec<(usize, Command)>), anyhow::Error> {
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
                Err(err) => return Err(err.into())
            };
        }

        let mut buffer = mem::take(&mut self.buffer);
        if still_open {
            self.buffer = buffer.split_off(buffer.iter().enumerate().rev().find(|(_, x)| **x == NEWLINE).map(|(idx, _)| idx + 1).unwrap_or(buffer.len()));
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
    RPCListener,
    _Machine(String),
    RPCConnection(usize),
    None,
}

#[derive(Debug)]
pub struct Daemon {
    event_key_storage: Vec<EventTarget>,
    global_config: GlobalConfig,
    machines: HashMap<String, VirtualMachine>,
    connections: Vec<Option<RPCConnection>>,
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
        log::debug!("Loading global config ({})", GLOBAL_CONFIG_LOCATION);
        let toml = std::fs::read_to_string(GLOBAL_CONFIG_LOCATION)?;
        let global_config = GlobalConfig::load(&toml)?;
        log::debug!("Creating vore daemon");
        let signals = Signals::new(&[SIGINT, SIGHUP])?;
        let handle = signals.handle();
        log::debug!("Bound signal handlers");
        let poller = Poller::new().context("Failed to make poller")?;
        let socket_path = PathBuf::from_str("/run/vore.sock")?;
        let rpc_listener = UnixListener::bind(&socket_path).context("Failed to bind vore socket")?;
        rpc_listener.set_nonblocking(true)?;
        log::debug!("Bound to /run/vore.sock");

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
        let new_key = self.add_target(EventTarget::RPCListener);
        self.poller.add(&self.rpc_listener, Event::readable(new_key))?;

        Ok(())
    }

    pub fn run(&mut self) -> Result<(), anyhow::Error> {
        loop {
            let res = self.wait().context("Got error while waiting for new notifications");
            match res {
                // Interrupted is uh "always" when we get a signal
                Err(err) if err.downcast_ref::<io::Error>().map(|x| x.kind() == io::ErrorKind::Interrupted).unwrap_or(false) => {
                    if !self.handle_exit_code()? {
                        break;
                    }
                }
                err => err?
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

    pub fn handle_command(&mut self, command: &Command) -> Result<AllResponses, anyhow::Error> {
        let resp = match &command.data {
            AllRequests::Info(_) => {
                rpc::InfoResponse {
                    name: "vore".to_string(),
                    version: format!("{}.{}.{}{}",
                                     env!("CARGO_PKG_VERSION_MAJOR"),
                                     env!("CARGO_PKG_VERSION_MINOR"),
                                     env!("CARGO_PKG_VERSION_PATCH"),
                                     option_env!("CARGO_PKG_VERSION_PRE").unwrap_or("")),
                }
                    .into_enum()
            }
            AllRequests::List(_) => {
                rpc::ListResponse {
                    items: self.machines.values().map(|x| x.info()).collect()
                }
                    .into_enum()
            }
            AllRequests::Load(val) => {
                let config = InstanceConfig::from_toml(&val.toml)?;
                let working_dir = val.working_directory.as_ref().cloned().unwrap_or_else(|| format!("/var/lib/vore/{}", config.name));
                let vm = VirtualMachine::new(config, &self.global_config, working_dir);
                let info = vm.info();
                self.mount_machine(vm);

                rpc::LoadResponse {
                    info,
                }
                    .into_enum()
            }
            AllRequests::Prepare(val) => {
                if let Some(machine) = self.machines.get_mut(&val.name) {
                    machine.prepare(true, false)?;
                } else {
                    anyhow::bail!("No machine with the name {} exists", val.name);
                }

                rpc::PrepareResponse {}.into_enum()
            }
            AllRequests::Start(_) => {
                anyhow::bail!("Unimplemented");
            }
            AllRequests::Stop(_) => {
                anyhow::bail!("Unimplemented");
            }
            AllRequests::Unload(_) => {
                anyhow::bail!("Unimplemented");
            }
        };

        Ok(resp)
    }

    pub fn handle_exit_code(&mut self) -> Result<bool, anyhow::Error> {
        for signal in self.signals.pending() {
            log::info!("Received signal {} ({})", signal_name(signal).unwrap_or("<unknown>"), signal);
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
                    EventTarget::RPCListener => {
                        self.poller.modify(&self.rpc_listener, Event::readable(event.key))?;
                        self.accept_rpc_connections()?;
                    }
                    EventTarget::_Machine(name) if self.machines.contains_key(&name) => {
                        if let Some(control_socket) = self.machines[&name].control_stream() {
                            self.poller.modify(control_socket, Event::readable(event.key))?;
                        }
                    }
                    EventTarget::RPCConnection(rpc_connection_id) if self.connections.get(rpc_connection_id).map(Option::is_some).unwrap_or(false) => {
                        let (still_open, mut commands) = if let Some(rpc_connection) = &mut self.connections[rpc_connection_id] {
                            let input_res = rpc_connection.handle_input(rpc_connection_id)?;
                            if input_res.0 {
                                self.poller.modify(&rpc_connection.stream, Event::readable(event.key))?;
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
                Err(err) => return Err(err)?
            };

            stream.set_nonblocking(true)?;

            let mut user: Option<String> = None;
            let ucred = unsafe {
                let mut ucred: libc::ucred = mem::zeroed();
                let mut length = size_of::<libc::ucred>() as u32;
                libc::getsockopt(stream.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED, (&mut ucred) as *mut _ as _, &mut length);
                let passwd = libc::getpwuid(ucred.uid);
                if !passwd.is_null() {
                    user = CStr::from_ptr((*passwd).pw_name).to_str().ok().map(|x| x.to_string())
                }

                ucred
            };

            let conn = RPCConnection {
                stream,
                address,
                buffer: vec![],
                uid: ucred.uid,
                user,
                pid: ucred.pid,
            };

            log::info!(
                "Got new RPC connection from {} (pid: {}, socket: {:?})",
                conn.user.as_ref().map_or_else(|| format!("uid:{}", conn.uid), |x| format!("{} ({})", x, conn.uid)),
                conn.pid,
                conn.address,
            );

            let id = self.add_rpc_connection(conn);
            let event_target = self.add_target(EventTarget::RPCConnection(id));
            self.poller.add(&self.connections[id].as_ref().unwrap().stream, Event::readable(event_target))?;
        }
    }

    pub fn wait(&mut self) -> Result<(), anyhow::Error> {
        self.poller.wait(&mut self.queue, Some(Duration::from_secs(5)))?;
        Ok(())
    }

    fn add_target(&mut self, event_target: EventTarget) -> usize {
        let id = self.event_key_storage.iter().enumerate().find(|(_, target)| target.eq(&&EventTarget::None));
        if let Some((id, _)) = id {
            self.event_key_storage[id] = event_target;
            return id;
        }

        let new_id = self.event_key_storage.len();
        self.event_key_storage.push(event_target);
        return new_id;
    }

    fn add_rpc_connection(&mut self, rpc_connection: RPCConnection) -> usize {
        let id = self.connections.iter().enumerate().find(|(_, target)| target.is_none());
        if let Some((id, _)) = id {
            self.connections[id] = Some(rpc_connection);
            return id;
        }

        let new_id = self.connections.len();
        self.connections.push(Some(rpc_connection));
        return new_id;
    }

    fn mount_machine(&mut self, vm: VirtualMachine) {
        let name = vm.name().to_string();
        self.machines.insert(name.clone(), vm);
    }
}