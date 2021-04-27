use std::collections::HashMap;
use vore_core::VirtualMachine;
use std::os::unix::net::UnixListener;

struct RPCConnection {}

struct DaemonState {
    machines: HashMap<String, VirtualMachine>,
    connections: Vec<RPCConnection>,
    rpc_listener: UnixListener,
}

impl DaemonState {
    pub fn wait() {

    }
}