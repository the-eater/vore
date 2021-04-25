use crate::{GlobalConfig, InstanceConfig, QemuCommandBuilder};
use std::option::Option::Some;
use std::path::PathBuf;
use std::process::{Child, Command};

#[derive(Debug)]
struct VirtualMachine {
    working_dir: PathBuf,
    config: InstanceConfig,
    process: Option<Child>,
}

impl VirtualMachine {
    pub fn new(config: InstanceConfig, working_dir: PathBuf) -> VirtualMachine {
        VirtualMachine {
            working_dir,
            config,
            process: None,
        }
    }

    pub fn start(&mut self, global_config: &GlobalConfig) -> Result<(), anyhow::Error> {
        if let Some(proc) = &mut self.process {
            if proc.try_wait()?.is_none() {
                return Ok(());
            }
        }

        let builder = QemuCommandBuilder::new(global_config, self.working_dir.clone())?;
        let cmd = builder.build(&self.config)?;

        let mut command = Command::new("qemu-system-x86_64");
        command.args(cmd);
        self.process = Some(command.spawn()?);

        Ok(())
    }
}
