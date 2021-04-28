use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const GLOBAL_CONFIG_LOCATION: &str = "/home/eater/projects/vored/config/global.toml";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GlobalConfig {
    pub qemu: GlobalQemuConfig,
    pub uefi: HashMap<String, GlobalUefiConfig>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GlobalQemuConfig {
    pub script: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all(deserialize = "kebab-case"))]
pub struct GlobalUefiConfig {
    pub template: String,
    pub boot_code: String,
}

impl GlobalConfig {
    pub fn load(toml: &str) -> Result<GlobalConfig, anyhow::Error> {
        toml::from_str(toml).context("Failed to parse toml for global config")
    }
}
