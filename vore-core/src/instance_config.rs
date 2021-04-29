use anyhow::{Context, Error};
use config::{Config, File, FileFormat, Value};
use serde::de::{Visitor};
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;
use std::fmt::{Debug, Display, Formatter};
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
    pub pulse: PulseConfig,
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

        instance_config.looking_glass = LookingGlassConfig::from_table(
            config.get_table("looking-glass").unwrap_or_default(),
        )?;
        instance_config.scream = ScreamConfig::from_table(
            config.get_table("scream").unwrap_or_default(),
        )?;
        instance_config.spice =
            SpiceConfig::from_table(config.get_table("spice").unwrap_or_default())?;

        instance_config.pulse = PulseConfig::from_table(config.get_table("pulse").unwrap_or_default())?;

        if let Ok(features) = config.get::<Vec<String>>("machine.features") {
            for feature in features {
                match feature.as_str() {
                    "looking-glass" => instance_config.looking_glass.enabled = true,
                    "spice" => instance_config.spice.enabled = true,
                    "scream" => instance_config.scream.enabled = true,
                    "uefi" => instance_config.uefi.enabled = true,
                    "pulse" => instance_config.pulse.enabled = true,
                    _ => {}
                }
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
            vfio: vec![],
            looking_glass: Default::default(),
            scream: Default::default(),
            pulse: Default::default(),
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
    ) -> Result<ScreamConfig, anyhow::Error> {
        let mut cfg = ScreamConfig::default();
        if let Some(enabled) = table.get("enabled").cloned() {
            cfg.enabled = enabled.into_bool()?;
        }

        if let Some(mem_path) = table.get("mem-path").cloned() {
            cfg.mem_path = mem_path.into_str()?;
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
    ) -> Result<LookingGlassConfig, anyhow::Error> {
        let mut cfg = LookingGlassConfig::default();

        if let Some(enabled) = table.get("enabled").cloned() {
            cfg.enabled = enabled.into_bool()?;
        }

        if let Some(mem_path) = table.get("mem-path").cloned() {
            cfg.mem_path = mem_path.into_str()?;
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
    pub read_only: bool,
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
                path.starts_with("/dev") | path.ends_with(".iso") => "raw",
                path.ends_with(".qcow2") => "qcow2",
                _ => return Err(anyhow::Error::msg("Can't figure out from path what type of disk driver should be used"))
            }).to_string()
        };

        let preset = table.get("preset")
            .cloned()
            .context("Every disk should have a preset set")?
            .into_str()?;

        let read_only = table.get("read-only")
            .cloned()
            .map(|x| x.into_bool())
            .transpose()
            .context("Failed to read read-only as boolean from config")?
            .unwrap_or(false);

        let disk = DiskConfig {
            disk_type,
            preset,
            path,
            read_only,
        };

        Ok(disk)
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct VfioConfig {
    pub address: PCIAddress,
    pub vendor: Option<u32>,
    pub device: Option<u32>,
    pub index: u32,
    pub graphics: bool,
    pub multifunction: bool,
}

pub fn read_pci_ids(addr: &PCIAddress) -> Result<(u32, u32), anyhow::Error> {
    let device = std::fs::read_to_string(format!("/sys/bus/pci/devices/{:#}/device", addr))
        .with_context(|| {
            format!(
                "Failed to read the device id of PCI device at {:#} ({})",
                addr,
                format!("/sys/bus/pci/devices/{:#}/device", addr)
            )
        })?;
    let found_device = u32::from_str_radix(device.trim_start_matches("0x").trim_end(), 16)?;

    let vendor = std::fs::read_to_string(format!("/sys/bus/pci/devices/{:#}/vendor", addr))
        .with_context(|| {
            format!(
                "Failed to read the vendor id of PCI device at {:#} ({})",
                addr,
                format!("/sys/bus/pci/devices/{:#}/vendor", addr)
            )
        })?;
    let found_vendor = u32::from_str_radix(vendor.trim_start_matches("0x").trim_end(), 16)?;

    Ok((found_vendor, found_device))
}

impl VfioConfig {
    pub fn from_table(table: HashMap<String, Value>) -> Result<VfioConfig, anyhow::Error> {
        let mut address = table
            .get("addr")
            .or_else(|| table.get("address"))
            .cloned()
            .map(|x| PCIAddress::from_str(&x.into_str()?))
            .transpose()?;

        let vendor = table
            .get("vendor")
            .cloned()
            .map(|x| x.into_int().map(|x| x as u32))
            .transpose()?;
        let device = table
            .get("device")
            .cloned()
            .map(|x| x.into_int().map(|x| x as u32))
            .transpose()?;
        let index = table
            .get("index")
            .cloned()
            .map(|x| x.into_int().map(|x| x as u32))
            .transpose()?
            .unwrap_or(0);

        let address = match (address, vendor, device) {
            (Some(addr), vendor, device) => {
                let (found_vendor, found_device) = read_pci_ids(&addr)?;

                if let Some(device) = device {
                    if device != found_device {
                        anyhow::bail!(
                            "VFIO expects a PCI device on address {} with the device id {:#04x} but found the id {:#04x} instead",
                            addr,
                            device,
                            found_device
                        )
                    }
                }

                if let Some(vendor) = vendor {
                    if vendor != found_vendor {
                        anyhow::bail!(
                            "VFIO expects a PCI device on address {} with the vendor id {:#04x} but found the id {:#04x} instead",
                            addr,
                            vendor,
                            found_vendor
                        )
                    }
                }

                addr
            }

            (None, Some(vendor), Some(device)) => {
                let mut counter = index;
                let mut items: Vec<(PCIAddress, u32, u32)> = vec![];

                for entry in std::fs::read_dir("/sys/bus/pci/devices")? {
                    let entry = entry?;
                    let file_name = entry.file_name();
                    let addr_name = file_name
                        .to_str()
                        .ok_or_else(|| anyhow::anyhow!("Failed to parse PCI device name"))?;
                    let addr = PCIAddress::from_str(addr_name)?;
                    let (found_vendor, found_device) = read_pci_ids(&addr)?;
                    items.push((addr, found_vendor, found_device));
                }

                items.sort_by_key(|&(addr, _, _)| addr);

                for (addr, found_vendor, found_device) in items {
                    if found_vendor == vendor && found_device == device {
                        if counter == 0 {
                            address = Some(addr);
                            break;
                        }

                        counter -= 1;
                    }
                }

                if let Some(address) = address {
                    address
                } else {
                    anyhow::bail!(
                        "Can't find {}th PCI device with vendor id {:#04x} and device id {:#04x}",
                        index + 1,
                        vendor,
                        device
                    )
                }
            }

            _ => anyhow::bail!("VFIO element needs either vendor and device or address to be set"),
        };

        let mut cfg = VfioConfig {
            address,
            vendor: None,
            device: None,
            index: 0,
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
pub struct PulseConfig {
    pub enabled: bool,
}

impl PulseConfig {
    pub fn from_table(table: HashMap<String, Value>) -> Result<PulseConfig, anyhow::Error> {
        let mut cfg = PulseConfig {
            enabled: false,
        };

        if let Some(enabled) = table.get("enabled").cloned() {
            cfg.enabled = enabled.into_bool()?;
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
            socket_path: "".to_string(),
        };

        if let Some(enabled) = table.get("enabled").cloned() {
            cfg.enabled = enabled.into_bool()?;
        }

        if let Some(socket_path) = table.get("socket-path").cloned() {
            cfg.socket_path = socket_path.into_str()?;
        }

        Ok(cfg)
    }
}

#[derive(Default, Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
pub struct PCIAddress {
    domain: u32,
    bus: u8,
    slot: u8,
    func: u8,
}

impl<'de> Deserialize<'de> for PCIAddress {
    fn deserialize<D>(deserializer: D) -> Result<Self, <D as Deserializer<'de>>::Error>
        where
            D: Deserializer<'de>,
    {
        struct X;
        impl Visitor<'_> for X {
            type Value = String;

            fn expecting(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("Expecting a string")
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E> where
                E: de::Error, {
                Ok(v.to_string())
            }

            fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(v)
            }
        }

        let x = deserializer.deserialize_string(X)?;
        Ok(PCIAddress::from_str(&x).map_err(|x| de::Error::custom(x))?)
    }
}

impl Serialize for PCIAddress {
    fn serialize<S>(&self, serializer: S) -> Result<<S as Serializer>::Ok, <S as Serializer>::Error>
        where
            S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl PCIAddress {
    fn to_string(&self) -> String {
        format!(
            "{:04x}:{:02x}:{:02x}.{:x}",
            self.domain, self.bus, self.slot, self.func
        )
    }
}

impl Debug for PCIAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("PCIAddress(")?;
        if f.alternate() && self.domain == 0 {
            f.write_str(&format!("{:04x}:", self.domain))?;
        }

        f.write_str(&format!(
            "{:02x}:{:02x}.{:x}",
            self.bus, self.slot, self.func
        ))?;
        f.write_str(")")
    }
}

impl Display for PCIAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if f.alternate() && self.domain == 0 {
            f.write_str(&format!("{:04x}:", self.domain))?;
        }

        f.write_str(&format!(
            "{:02x}:{:02x}.{:x}",
            self.bus, self.slot, self.func
        ))
    }
}

impl FromStr for PCIAddress {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut rev = s.rsplit(":");
        let mut addr = PCIAddress::default();

        if let Some(slot_and_func) = rev.next() {
            let mut splitter = slot_and_func.split(".");

            if let Some(slot) = splitter.next() {
                addr.slot = u8::from_str_radix(slot, 16)?;
            }

            if let Some(func) = splitter.next() {
                addr.func = u8::from_str_radix(func, 16)?;
            }
        }

        if let Some(bus) = rev.next() {
            addr.bus = u8::from_str_radix(bus, 16)?;
        }

        if let Some(domain) = rev.next() {
            addr.domain = u32::from_str_radix(domain, 16)?;
        }

        Ok(addr)
    }
}

#[cfg(test)]
mod tests {
    use crate::PCIAddress;
    use std::str::FromStr;

    #[test]
    fn test_input_and_output_are_same() {
        assert_eq!(
            PCIAddress::from_str("0000:00:00.1")
                .expect("Failed to parse correct string")
                .to_string(),
            "0000:00:00.1"
        );

        assert_eq!(
            PCIAddress::from_str("0000:00:01.0")
                .expect("Failed to parse correct string")
                .to_string(),
            "0000:00:01.0"
        );
    }
}
