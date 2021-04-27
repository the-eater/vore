mod daemon;

use std::path::PathBuf;
use vore_core::{GlobalConfig, InstanceConfig, VirtualMachine};

fn main() {
    let cfg = InstanceConfig::from_toml(include_str!("../../config/example.toml")).unwrap();
    let global = GlobalConfig::load(include_str!("../../config/global.toml")).unwrap();
    let mut vm = VirtualMachine::new(cfg, &global, PathBuf::from("/home/eater/.local/vore/win10"));
    vm.prepare(true, false).unwrap();
    vm.start().unwrap();
    vm.wait_till_stopped().unwrap();
    vm.stop_now().unwrap()
}
