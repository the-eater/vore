use lazy_static::lazy_static;

#[derive(Copy, Clone, Debug)]
pub struct Cpu {
    pub id: usize,
    pub package: usize,
    pub die: usize,
    pub core: usize,
    pub layer_0: Option<usize>,
    pub layer_1: Option<usize>,
    pub layer_2: Option<usize>,
    pub layer_3: Option<usize>,
}

lazy_static! {
    static ref CPUS: Box<[Cpu]> = get_cpus().into_boxed_slice();
    static ref CPU_LIST: CpuList = CpuList { list: &*CPUS };
}

pub fn get_cpus() -> Vec<Cpu> {
    if cfg!(target_os = "linux") {
        crate::cpu_list::linux::get_cpus()
    } else {
        unimplemented!();
    }
}

#[derive(Copy, Clone, Debug)]
pub struct CpuList {
    list: &'static [Cpu],
}

impl CpuList {
    pub fn _get() -> CpuList {
        *CPU_LIST
    }

    pub fn _amount() -> usize {
        CPU_LIST.len()
    }

    pub fn _load() -> CpuListOwned {
        CpuListOwned { list: get_cpus() }
    }

    pub fn adjacent(amount: usize) -> Option<&'static [Cpu]> {
        CPU_LIST.get_adjacent(amount)
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn _as_slice(&self) -> &[Cpu] {
        self.list
    }

    pub fn get_adjacent(&self, amount: usize) -> Option<&[Cpu]> {
        if self.len() < amount {
            None
        } else {
            Some(&self.list[..amount])
        }
    }
}

#[derive(Clone, Debug)]
pub struct CpuListOwned {
    list: Vec<Cpu>,
}

impl CpuListOwned {}

#[cfg(target_os = "linux")]
mod linux {
    use crate::cpu_list::Cpu;
    use std::fs::read_to_string;
    use std::str::FromStr;

    pub fn get_cpus() -> Vec<Cpu> {
        let cpu = std::fs::read_dir("/sys/devices/system/cpu")
            .expect("Failed to read /sys/devices/system/cpu, no /sys mounted?");
        let mut cpus = vec![];
        for cpu_dir in cpu {
            let cpu_dir = cpu_dir.unwrap();
            let file_name = cpu_dir.file_name();
            let cpu_name = file_name.to_str().unwrap();
            if cpu_name.starts_with("cpu") && cpu_name[3..].chars().all(|x| x.is_ascii_digit()) {
                let cpu_id = usize::from_str(&cpu_name[3..]).unwrap();
                let topology = cpu_dir.path();
                let read_id = |name: &str| -> Option<usize> {
                    let mut path = topology.clone();
                    path.push(name);
                    let id_str = read_to_string(&path).ok()?;
                    usize::from_str(id_str.trim_end()).ok()
                };
                cpus.push(Cpu {
                    id: cpu_id,
                    package: read_id("topology/physical_package_id").unwrap(),
                    die: read_id("topology/die_id").unwrap(),
                    core: read_id("topology/core_id").unwrap(),
                    layer_0: read_id("cache/index0/id"),
                    layer_1: read_id("cache/index1/id"),
                    layer_2: read_id("cache/index2/id"),
                    layer_3: read_id("cache/index3/id"),
                })
            }
        }

        cpus.sort_by_key(|x| {
            (
                x.package, x.die, x.layer_3, x.layer_2, x.layer_1, x.layer_0, x.core, x.id,
            )
        });

        cpus
    }
}