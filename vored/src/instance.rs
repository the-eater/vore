use std::fmt::{Display, Formatter};
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

#[derive(Debug, Clone)]
pub struct ArgumentList {
    items: Vec<Argument>,
}

impl Display for ArgumentList {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let item = self.items.iter().fold(String::new(), |mut x, item| {
            if x.len() > 0 {
                x.push_str(" ")
            }
            x.push_str(item.as_str());
            x
        });
        f.write_str(&item)
    }
}

impl ArgumentList {
    pub fn new(command: &'static str) -> ArgumentList {
        ArgumentList {
            items: vec![Argument::Borrowed(command)],
        }
    }

    pub fn pair(&mut self, key: &'static str, value: String) {
        self.push(key);
        self.push(value);
    }
}

pub trait PushArgument<T> {
    fn push(&mut self, argument: T);
    fn push_pair(&mut self, key: T, value: T) {
        self.push(key);
        self.push(value);
    }
}

impl PushArgument<String> for ArgumentList {
    fn push(&mut self, argument: String) {
        self.items.push(Argument::Owned(argument))
    }
}

impl PushArgument<&'static str> for ArgumentList {
    fn push(&mut self, argument: &'static str) {
        self.items.push(Argument::Borrowed(argument))
    }
}

#[derive(Debug, Clone)]
enum Argument {
    Owned(String),
    Borrowed(&'static str),
}

impl Argument {
    fn as_str(&self) -> &str {
        match self {
            Argument::Owned(owned) => &owned,
            Argument::Borrowed(borrowed) => borrowed,
        }
    }
}

pub fn build_qemu_command(config: &InstanceConfig) -> ArgumentList {
    let mut cmd = ArgumentList::new("qemu-system-x86_64");
    cmd.pair("-name", format!("guest={},debug-threads=on", config.name));

    cmd.push("-S");
    cmd.push("-no-user-config");
    cmd.push("-no-defaults");
    cmd.push("-no-shutdown");

    if config.kvm {
        cmd.push("-enable-kvm");
    }

    cmd.pair("-m", config.memory.to_string());

    if config.uefi.enabled {
        // OVMF will hang if S3 is not disabled
        // disable S4 too, since libvirt does that 🤷
        // https://bugs.archlinux.org/task/59465#comment172528
        cmd.push_pair("-global", "ICH9-LPC.disable_s3=1");
        cmd.push_pair("-global", "ICH9-LPC.disable_s4=1");
    }

    cmd.push_pair("-rtc", "driftfix=slew");
    cmd.push_pair("-serial", "stdio");

    #[cfg(any(target_arch = "x86_64", target_arch = "i686"))]
    {
        cmd.push_pair("-global", "kvm-pit.lost_tick_policy=discard")
    }

    cmd.push("-no-hpet");
    cmd.push_pair("-boot", "strict=on");

    cmd.pair(
        "-smp",
        format!(
            "{},sockets={},dies={},cores={},threads={}",
            config.cpu.amount,
            config.cpu.sockets,
            config.cpu.dies,
            config.cpu.cores,
            config.cpu.threads
        ),
    );

    cmd.push_pair("-msg", "timestamp=on");

    cmd
}

#[derive(Debug)]
pub struct Qemu {
    process: Option<Child>,
}
