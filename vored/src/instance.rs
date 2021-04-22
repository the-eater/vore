use std::process::Child;
use vore_core::InstanceConfig;

#[derive(Debug)]
pub struct Instance {
    config: InstanceConfig,
    qemu: Option<Qemu>,
}

impl Instance {
    pub fn from_config(config: InstanceConfig) -> Instance {
        Instance { config, qemu: None }
    }

    pub fn spawn_qemu(&self) -> Result<(), anyhow::Error> {
        Ok(())
    }
}

#[derive(Debug)]
pub struct Qemu {
    process: Option<Child>,
}
