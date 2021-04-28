mod client;

use vore_core::{init_logging};
use crate::client::Client;
use clap::{App, ArgMatches};
use std::fs;
use anyhow::Context;
use std::option::Option::Some;

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
    let client = Client::connect(matches.value_of("vored-socket").unwrap())?;

    let mut vore = VoreApp {
        client
    };

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

        ("daemon", Some(args)) => {
            match args.subcommand() {
                ("version", _) => {
                    vore.daemon_version()?;
                }

                (s, _) => {
                    log::error!("Subcommand daemon.{} not implemented", s);
                }
            }
        }

        (s, _) => {
            log::error!("Subcommand {} not implemented", s);
        }
    }

    Ok(())
}

struct LoadVMOptions {
    config: String,
    cdroms: Vec<String>,
    save: bool,
}

fn get_load_vm_options(args: &ArgMatches) -> anyhow::Result<LoadVMOptions> {
    let vm_config_path = args.value_of("vm-config").unwrap();
    let config = fs::read_to_string(vm_config_path)
        .with_context(|| format!("Failed to read vm config at {}", vm_config_path))?;

    Ok(LoadVMOptions {
        config,
        cdroms: args.values_of("cdrom").map_or(vec![], |x| x.map(|x| x.to_string()).collect::<Vec<_>>()),
        save: args.is_present("save"),
    })
}

struct VoreApp {
    client: Client,
}

impl VoreApp {
    fn get_vm_name(&mut self, args: &ArgMatches) -> anyhow::Result<String> {
        if let Some(vm_name) = args.value_of("vm-name") {
            Ok(vm_name.to_string())
        } else {
            let mut items = self.client.list_vms()?;
            match (items.len(), items.pop()) {
                (amount, Some(x)) if amount == 1 => return Ok(x.name),
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

        let vm_info = self.client.load_vm(&vm_options.config, vm_options.save, vm_options.cdroms)?;
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

    fn prepare(&mut self, args: &ArgMatches) -> anyhow::Result<()> {
        let name = self.get_vm_name(args)?;
        self.client.prepare(name, args.values_of("cdrom").map_or(vec![], |x| x.map(|x| x.to_string()).collect::<Vec<_>>()))?;
        Ok(())
    }
}
