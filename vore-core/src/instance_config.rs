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
    pub vfio: Vec<VfioConfig>,
    pub looking_glass: LookingGlassConfig,
    pub scream: ScreamConfig,
    pub spice: SpiceConfig,
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

        if let Ok(uefi) = config.get_table("uefi") {
            instance_config.uefi.apply_table(uefi)?;
        }

        if let Ok(vfio) = config.get::<Value>("vfio") {
            let arr = vfio.into_array().context("vfio should be an array")?;
            for (i, disk) in arr.into_iter().enumerate() {
                let table = disk
                    .into_table()
                    .with_context(|| format!("vfio[{}] should be a table", i))?;
                instance_config.vfio.push(VfioConfig::from_table(table)?);
            }
        }

        if let Ok(looking_glass) = config.get_table("looking-glass") {
            instance_config.looking_glass =
                LookingGlassConfig::from_table(looking_glass, &instance_config.name)?;
        }

        if let Ok(scream) = config.get_table("scream") {
            instance_config.scream = ScreamConfig::from_table(scream, &instance_config.name)?;
        }

        if let Ok(scream) = config.get_table("spice") {
            instance_config.spice = SpiceConfig::from_table(scream)?;
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
            vfio: vec![],
            looking_glass: Default::default(),
            scream: Default::default(),
            spice: Default::default(),
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

impl UefiConfig {
    fn apply_table(&mut self, table: HashMap<String, Value>) -> Result<(), anyhow::Error> {
        if let Some(enabled) = table
            .get("enabled")
            .cloned()
            .map(|x| x.into_bool().context("eufi.enabled should be a boolean"))
            .transpose()?
        {
            self.enabled = enabled
        }

        Ok(())
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct ScreamConfig {
    pub enabled: bool,
    pub mem_path: String,
    pub buffer_size: u64,
}

impl ScreamConfig {
    pub fn from_table(
        table: HashMap<String, Value>,
        name: &str,
    ) -> Result<ScreamConfig, anyhow::Error> {
        let mut cfg = ScreamConfig::default();
        if let Some(enabled) = table.get("enabled").cloned() {
            cfg.enabled = enabled.into_bool()?;
        }

        if let Some(mem_path) = table.get("mem-path").cloned() {
            cfg.mem_path = mem_path.into_str()?;
        } else {
            cfg.mem_path = format!("/dev/shm/{}-scream", name);
        }

        if let Some(buffer_size) = table.get("buffer-size").cloned() {
            cfg.buffer_size = buffer_size.into_int()? as u64;
        }

        Ok(cfg)
    }
}

impl Default for ScreamConfig {
    fn default() -> Self {
        ScreamConfig {
            enabled: false,
            mem_path: "".to_string(),
            buffer_size: 2097152,
        }
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct LookingGlassConfig {
    pub enabled: bool,
    pub mem_path: String,
    pub buffer_size: u64,
    pub width: u64,
    pub height: u64,
    pub bit_depth: u64,
}

impl Default for LookingGlassConfig {
    fn default() -> Self {
        LookingGlassConfig {
            enabled: false,
            mem_path: "".to_string(),
            buffer_size: 0,
            width: 1920,
            height: 1080,
            bit_depth: 8,
        }
    }
}

impl LookingGlassConfig {
    pub fn calc_buffer_size_from_screen(&mut self) {
        // https://forum.level1techs.com/t/solved-what-is-max-frame-size-determined-by/170312/4
        //
        // required memory size is
        //
        // height * width * 4 * 2 + 2mb
        //
        // And shared memory size needs to be a power off 2
        //
        let mut minimum_needed =
            self.width * self.height * (((self.bit_depth * 4) as f64 / 8f64).ceil() as u64);

        // 2 frames
        minimum_needed *= 2;

        // Add additional 2mb
        minimum_needed += 2 * 1024 * 1024;

        let mut i = 1;
        let mut buffer_size = 1;
        while buffer_size < minimum_needed {
            i += 1;
            buffer_size = 2u64.pow(i);
        }

        self.buffer_size = buffer_size;
    }

    pub fn from_table(
        table: HashMap<String, Value>,
        name: &str,
    ) -> Result<LookingGlassConfig, anyhow::Error> {
        let mut cfg = LookingGlassConfig::default();

        if let Some(enabled) = table.get("enabled").cloned() {
            cfg.enabled = enabled.into_bool()?;
        }

        if let Some(mem_path) = table.get("mem-path").cloned() {
            cfg.mem_path = mem_path.into_str()?;
        } else {
            cfg.mem_path = format!("/dev/shm/{}-looking-glass", name);
        }

        match (table.get("buffer-size").cloned(), table.get("width").cloned(), table.get("height").cloned()) {
            (Some(buffer_size), None, None) => {
                cfg.buffer_size = buffer_size.into_int()? as u64;
            }

            (None, Some(width), Some(height)) => {
                let width = width.into_int()? as u64;
                let height = height.into_int()? as u64;
                let bit_depth = table.get("bit-depth").cloned().map_or(Ok(cfg.bit_depth), |x| x.into_int().map(|x| x as u64))?;
                cfg.bit_depth = bit_depth;
                cfg.width = width;
                cfg.height = height;
                cfg.calc_buffer_size_from_screen();
            }

            (None, None, None) => {
                cfg.calc_buffer_size_from_screen()
            }

            _ => anyhow::bail!("for looking-glass either width and height need to be set or buffer-size should be set")
        }

        Ok(cfg)
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct DiskConfig {
    pub disk_type: String,
    pub preset: String,
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

        let preset = table.get("preset").cloned().context("gamer")?.into_str()?;

        let disk = DiskConfig {
            disk_type,
            preset,
            path,
        };

        // TODO: Add blockdev details

        Ok(disk)
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct VfioConfig {
    pub slot: String,
    pub graphics: bool,
    pub multifunction: bool,
}

impl VfioConfig {
    pub fn from_table(table: HashMap<String, Value>) -> Result<VfioConfig, anyhow::Error> {
        let slot = table
            .get("slot")
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("vfio table needs a slot"))?
            .into_str()?;
        let mut cfg = VfioConfig {
            slot,
            graphics: false,
            multifunction: false,
        };

        if let Some(graphics) = table.get("graphics").cloned() {
            cfg.graphics = graphics.into_bool()?;
        }

        if let Some(multifunction) = table.get("multifunction").cloned() {
            cfg.multifunction = multifunction.into_bool()?;
        }

        Ok(cfg)
    }
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct SpiceConfig {
    pub enabled: bool,
    pub socket_path: String,
}

impl SpiceConfig {
    pub fn from_table(table: HashMap<String, Value>) -> Result<SpiceConfig, anyhow::Error> {
        let mut cfg = SpiceConfig {
            enabled: false,
            socket_path: "/tmp/win10.sock".to_string(),
        };

        if let Some(enabled) = table.get("enabled").cloned() {
            cfg.enabled = enabled.into_bool()?;
        }

        Ok(cfg)
    }
}
