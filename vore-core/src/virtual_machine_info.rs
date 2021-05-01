use std::fmt::{Display, Formatter};
use std::fmt;
use crate::InstanceConfig;
use std::path::PathBuf;
use serde::{Deserialize, Serialize};

#[derive(Eq, PartialEq, Copy, Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VirtualMachineState {
    Loaded,
    Prepared,
    Stopped,
    Paused,
    Running,
}

impl Display for VirtualMachineState {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            VirtualMachineState::Loaded => write!(f, "loaded"),
            VirtualMachineState::Prepared => write!(f, "prepared"),
            VirtualMachineState::Stopped => write!(f, "stopped"),
            VirtualMachineState::Paused => write!(f, "paused"),
            VirtualMachineState::Running => write!(f, "running")
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct VirtualMachineInfo {
    pub name: String,
    pub working_dir: PathBuf,
    pub config: InstanceConfig,
    pub state: VirtualMachineState,
    pub quit_after_shutdown: bool,
}