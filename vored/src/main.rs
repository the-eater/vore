use crate::instance::build_qemu_command;
use vore_core::InstanceConfig;

mod instance;

fn main() {
    let cfg = InstanceConfig::from_toml(include_str!("../../config/example.toml")).unwrap();
    println!("Hello, world! {}", build_qemu_command(&cfg));
}
