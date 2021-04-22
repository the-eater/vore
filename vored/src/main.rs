use vore_core::{build_qemu_command, GlobalConfig, InstanceConfig};

mod instance;

fn main() {
    let cfg = InstanceConfig::from_toml(include_str!("../../config/example.toml")).unwrap();
    let global = GlobalConfig::load(include_str!("../../config/global.toml")).unwrap();
    println!("Hello, world! {:?}", build_qemu_command(&cfg, &global));
    print!("hello world {:#?}", global);
}
