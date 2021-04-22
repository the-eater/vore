use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GlobalConfig {
    pub qemu: GlobalQemuConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GlobalQemuConfig {
    pub default: Vec<String>,
    pub arch: HashMap<String, Vec<String>>,
    pub uefi: Vec<String>,
}

impl GlobalConfig {
    pub fn load(toml: &str) -> Result<GlobalConfig, anyhow::Error> {
        toml::from_str(toml).context("Failed to parse toml for global config")
    }
}
