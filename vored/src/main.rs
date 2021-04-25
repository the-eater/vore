use std::path::PathBuf;
use std::process::Command;
use vore_core::{GlobalConfig, InstanceConfig, QemuCommandBuilder};

fn main() {
    let cfg = InstanceConfig::from_toml(include_str!("../../config/example.toml")).unwrap();
    println!("CONFIG:\n{:#?}", cfg);
    let global = GlobalConfig::load(include_str!("../../config/global.toml")).unwrap();
    let builder =
        QemuCommandBuilder::new(&global, PathBuf::from("/home/eater/.lib/vore/win10")).unwrap();
    let command = builder.build(&cfg).unwrap();
    // .iter()
    // .map(|x| format!("'{}'", x))
    // .collect::<Vec<_>>()
    // .join(" ");

    Command::new("qemu-system-x86_64")
        .args(command)
        .spawn()
        .unwrap()
        .wait()
        .unwrap();
}
