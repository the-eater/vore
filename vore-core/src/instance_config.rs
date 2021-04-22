use anyhow::{Context, Error};
use config::{Config, File, FileFormat, Value};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct InstanceConfig {
    pub name: String,
    pub arch: String,
    pub chipset: String,
    pub kvm: bool,
    pub memory: u64,
    pub cpu: CpuConfig,
    pub disks: Vec<DiskConfig>,
    pub uefi: UefiConfig,
    pub looking_glass: LookingGlassConfig,
    pub scream: ScreamConfig,
}

impl InstanceConfig {
    pub fn from_toml(toml: &str) -> Result<Self, anyhow::Error> {
        let toml = Config::new().with_merged(File::from_str(toml, FileFormat::Toml))?;
        Self::from_config(toml)
    }

    pub fn from_config(config: Config) -> Result<InstanceConfig, anyhow::Error> {
        let mut instance_config = InstanceConfig::default();
        if let Ok(name) = config.get_str("machine.name") {
            instance_config.name = name
        }

        if let Ok(kvm) = config.get::<Value>("machine.kvm") {
            instance_config.kvm = kvm.into_bool().context("machine.kvm should be a boolean")?;
        }

        if let Ok(mem) = config.get::<Value>("machine.memory") {
            let mem = mem
                .into_str()
                .context("machine.memory should be a string or number")?;
            instance_config.memory = parse_size(&mem)?;
        }

        if let Ok(cpu) = config.get_table("cpu") {
            instance_config.cpu.apply_table(cpu)?
        }

        if let Ok(disks) = config.get::<Value>("disk") {
            let arr = disks.into_array().context("disk should be an array")?;
            for (i, disk) in arr.into_iter().enumerate() {
                let table = disk
                    .into_table()
                    .with_context(|| format!("disk[{}] should be a table", i))?;
                instance_config.disks.push(DiskConfig::from_table(table)?);
            }
        }

        Ok(instance_config)
    }
}

impl Default for InstanceConfig {
    fn default() -> Self {
        InstanceConfig {
            name: "vore".to_string(),
            arch: std::env::consts::ARCH.to_string(),
            chipset: "q35".to_string(),
            kvm: true,
            // 2 GB
            memory: 2 * 1024 * 1024 * 1024,
            cpu: Default::default(),
            disks: vec![],
            uefi: Default::default(),
            looking_glass: Default::default(),
            scream: Default::default(),
        }
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct CpuConfig {
    pub amount: u64,
    pub cores: u64,
    pub threads: u64,
    pub dies: u64,
    pub sockets: u64,
}

impl Default for CpuConfig {
    fn default() -> Self {
        CpuConfig {
            amount: 2,
            cores: 1,
            threads: 2,
            dies: 1,
            sockets: 1,
        }
    }
}

fn get_positive_number_from_table(
    table: &HashMap<String, Value>,
    key: &str,
    prefix: &str,
) -> Result<Option<u64>, Error> {
    table
        .get(key)
        .cloned()
        .map(|x| {
            x.into_int()
                .with_context(|| format!("Failed to parse {}.{} as number", prefix, key))
                .and_then(|x| {
                    Some(x)
                        .filter(|x| !x.is_negative())
                        .map(|x| x as u64)
                        .ok_or_else(|| {
                            anyhow::Error::msg(format!("{}.{} can't be negative", prefix, key))
                        })
                })
        })
        .transpose()
}

impl CpuConfig {
    fn apply_table(&mut self, table: HashMap<String, Value>) -> Result<(), anyhow::Error> {
        if let Some(amount) = get_positive_number_from_table(&table, "amount", "cpu")? {
            self.amount = amount;
        }

        if let Some(cores) = get_positive_number_from_table(&table, "cores", "cpu")? {
            self.cores = cores;
        }

        if let Some(threads) = get_positive_number_from_table(&table, "threads", "cpu")? {
            self.threads = threads;
        }

        if let Some(dies) = get_positive_number_from_table(&table, "dies", "cpu")? {
            self.dies = dies;
        }

        if let Some(sockets) = get_positive_number_from_table(&table, "sockets", "cpu")? {
            self.sockets = sockets;
        }

        if !table.contains_key("amount") {
            self.amount = self.sockets * self.dies * self.cores * self.threads;
        } else {
            if table
                .keys()
                .any(|x| ["cores", "sockets", "dies", "threads"].contains(&x.as_str()))
            {
                let calc_amount = self.sockets * self.dies * self.cores * self.threads;
                if self.amount != calc_amount {
                    Err(anyhow::Error::msg(format!("Amount of cpu's ({}) from sockets ({}), dies ({}), cores ({}) and threads ({}) differs from specified ({}) cpu's", calc_amount, self.sockets, self.dies, self.cores, self.threads, self.amount)))?;
                }
            } else {
                if (self.amount % 2) == 0 {
                    self.cores = self.amount / 2;
                } else {
                    self.threads = 1;
                    self.cores = self.amount;
                }
            }
        }

        Ok(())
    }
}

fn parse_size(orig_input: &str) -> Result<u64, anyhow::Error> {
    let input = orig_input.to_string().to_lowercase().replace(" ", "");
    let mut input = input.strip_suffix("b").unwrap_or(&input);
    let mut modifier: u64 = 1;

    if input.chars().last().unwrap_or('_').is_alphabetic() {
        modifier = match input.chars().last().unwrap() {
            'k' => {
                return Err(anyhow::Error::msg(
                    "size can only be specified in megabytes or larger",
                ));
            }
            'm' => 1,
            'g' => 1024,
            't' => 1024 * 1024,
            _ => {
                return Err(anyhow::Error::msg(format!(
                    "'{}' is not a valid size",
                    orig_input
                )));
            }
        };

        input = &input[..input.len() - 1];
    }

    if input.len() == 0 {
        return Err(anyhow::Error::msg(format!(
            "'{}' is not a valid size",
            orig_input
        )));
    }

    u64::from_str(input)
        .context(format!("'{}' is not a valid size", orig_input))
        .map(|x| x * modifier)
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct UefiConfig {
    pub enabled: bool,
}

impl Default for UefiConfig {
    fn default() -> Self {
        UefiConfig { enabled: false }
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct ScreamConfig {
    pub enabled: bool,
}

impl Default for ScreamConfig {
    fn default() -> Self {
        ScreamConfig { enabled: false }
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct LookingGlassConfig {
    pub enabled: bool,
}

impl Default for LookingGlassConfig {
    fn default() -> Self {
        LookingGlassConfig { enabled: false }
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct DiskConfig {
    pub disk_type: String,
    pub path: String,
}

impl DiskConfig {
    pub fn from_table(table: HashMap<String, Value>) -> Result<DiskConfig, anyhow::Error> {
        let path = table
            .get("path")
            .cloned()
            .ok_or_else(|| anyhow::Error::msg("Disk needs a path"))?
            .into_str()
            .context("Disk path must be a string")?;

        let disk_type = if let Some(disk_type) = table.get("type").cloned() {
            disk_type.into_str()?
        } else {
            (kiam::when! {
                path.starts_with("/dev") => "raw",
                path.ends_with(".qcow2") => "qcow2",
                _ => return Err(anyhow::Error::msg("Can't figure out from path what type of disk driver should be used"))
            }).to_string()
        };

        let disk = DiskConfig { disk_type, path };

        // TODO: Add blockdev details

        Ok(disk)
    }
}
