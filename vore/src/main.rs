mod client;

use crate::client::Client;
use anyhow::Context;
use clap::{App, ArgMatches};
use std::option::Option::Some;
use std::os::unix::process::CommandExt;
use std::process::Command;
use std::{fs, mem};
use vore_core::consts::VORE_SOCKET;
use vore_core::rpc::DiskPreset;
use vore_core::{init_logging, VirtualMachineInfo};

fn main() {
    init_logging();

    if let Err(err) = main_res() {
        println!("{:?}", err)
    }
}

fn main_res() -> anyhow::Result<()> {
    let yaml = clap::load_yaml!("../clap.yml");
    let app: App = App::from(yaml);
    let matches = app.get_matches();
    let client = Client::connect(matches.value_of("vored-socket").unwrap_or(VORE_SOCKET))?;

    let mut vore = VoreApp { client };

    match matches.subcommand() {
        ("load", Some(args)) => {
            vore.load(args)?;
        }

        ("list", Some(args)) => {
            vore.list(args)?;
        }

        ("prepare", Some(args)) => {
            vore.prepare(args)?;
        }

        ("start", Some(args)) => {
            vore.start(args)?;
        }

        ("stop", Some(args)) => {
            vore.stop(args)?;
        }

        ("looking-glass", Some(args)) => {
            vore.looking_glass(args)?;
        }

        ("daemon", Some(args)) => match args.subcommand() {
            ("version", _) => {
                vore.daemon_version()?;
            }

            (s, _) => {
                log::error!("Subcommand daemon.{} not implemented", s);
            }
        },

        ("disk", Some(args)) => match args.subcommand() {
            ("presets", _) => {
                vore.list_presets()?;
            }

            (s, _) => {
                log::error!("Subcommand disk.{} not implemented", s);
            }
        },

        (s, _) => {
            log::error!("Subcommand {} not implemented", s);
        }
    }

    Ok(())
}

struct LoadVirtualMachineOptions {
    config: String,
    cd_roms: Vec<String>,
    save: bool,
}

fn get_load_vm_options(args: &ArgMatches) -> anyhow::Result<LoadVirtualMachineOptions> {
    let vm_config_path = args.value_of("vm-config").unwrap();
    let config = fs::read_to_string(vm_config_path)
        .with_context(|| format!("Failed to read vm config at {}", vm_config_path))?;

    Ok(LoadVirtualMachineOptions {
        config,
        cd_roms: args
            .values_of("cdrom")
            .map_or(vec![], |x| x.map(|x| x.to_string()).collect::<Vec<_>>()),
        save: args.is_present("save"),
    })
}

struct VoreApp {
    client: Client,
}

impl VoreApp {
    fn get_vm_name(&mut self, args: &ArgMatches) -> anyhow::Result<String> {
        self.get_vm(args).map(|x| x.name)
    }

    pub fn get_vm(&mut self, args: &ArgMatches) -> anyhow::Result<VirtualMachineInfo> {
        let mut items = self.client.list_vms()?;
        if let Some(vm_name) = args.value_of("vm-name") {
            items
                .into_iter()
                .find(|x| x.name == vm_name)
                .with_context(|| format!("Couldn't find VM with the name '{}'", vm_name))
        } else {
            match (items.len(), items.pop()) {
                (amount, Some(x)) if amount == 1 => Ok(x),
                (0, None) => anyhow::bail!("There are no VM's loaded"),
                _ => anyhow::bail!("Multiple VM's are loaded, please specify one"),
            }
        }
    }

    fn daemon_version(&mut self) -> anyhow::Result<()> {
        let info = self.client.host_version()?;
        println!("{} ({})", info.version, info.name);
        Ok(())
    }

    fn load(&mut self, args: &ArgMatches) -> anyhow::Result<()> {
        let vm_options = get_load_vm_options(args)?;

        let vm_info =
            self.client
                .load_vm(&vm_options.config, vm_options.save, vm_options.cd_roms)?;
        log::info!("Loaded VM {}", vm_info.name);
        Ok(())
    }

    fn list(&mut self, _: &ArgMatches) -> anyhow::Result<()> {
        let items = self.client.list_vms()?;

        for info in items {
            println!("{}\t{}", info.name, info.state)
        }

        Ok(())
    }

    fn list_presets(&mut self) -> anyhow::Result<()> {
        let items = self.client.list_disk_presets()?;

        for DiskPreset { name, description } in items {
            println!("{}\t{}", name, description)
        }

        Ok(())
    }

    fn prepare(&mut self, args: &ArgMatches) -> anyhow::Result<()> {
        let name = self.get_vm_name(args)?;
        self.client.prepare(
            name,
            args.values_of("cdrom")
                .map_or(vec![], |x| x.map(|x| x.to_string()).collect::<Vec<_>>()),
        )?;
        Ok(())
    }

    fn start(&mut self, args: &ArgMatches) -> anyhow::Result<()> {
        let name = self.get_vm_name(args)?;
        self.client.start(
            name,
            args.values_of("cdrom")
                .map_or(vec![], |x| x.map(|x| x.to_string()).collect::<Vec<_>>()),
        )?;
        Ok(())
    }

    fn looking_glass(mut self, args: &ArgMatches) -> anyhow::Result<()> {
        let vm = self.get_vm(args)?;
        if !vm.config.looking_glass.enabled {
            anyhow::bail!("VM '{}' has no looking glass", vm.name);
        }

        let mut command = Command::new(
            std::env::var("LOOKING_GLASS").unwrap_or_else(|_| "looking-glass-client".to_string()),
        );
        if vm.config.spice.enabled {
            command.args(&["-c", &vm.config.spice.socket_path, "-p", "0"]);
        } else {
            command.args(&["-s", "no"]);
        }

        command.args(&["-f", &vm.config.looking_glass.mem_path]);
        command.args(
            args.values_of("looking-glass-args")
                .map_or(vec![], |x| x.into_iter().collect::<Vec<_>>()),
        );

        mem::drop(self);
        command.exec();

        Ok(())
    }

    fn stop(&mut self, args: &ArgMatches) -> anyhow::Result<()> {
        let name = self.get_vm_name(args)?;
        self.client.stop(name)?;
        Ok(())
    }
}
