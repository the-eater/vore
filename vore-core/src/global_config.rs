use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::CString;
use std::fs;
use std::fs::Permissions;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GlobalConfig {
    pub vore: GlobalVoreConfig,
    pub qemu: GlobalQemuConfig,
    pub uefi: HashMap<String, GlobalUefiConfig>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all(deserialize = "kebab-case"))]
pub struct GlobalVoreConfig {
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub unix_group_id: Option<libc::gid_t>,
}

impl GlobalVoreConfig {
    pub fn get_gid(&mut self) -> Result<Option<u32>, anyhow::Error> {
        if let Some(id) = self.unix_group_id {
            return Ok(Some(id));
        }

        let name = self.group.as_ref().cloned();

        name.map(|group_name| {
            let group_name_c = CString::new(group_name.as_str())?;
            Ok(unsafe {
                let group = libc::getgrnam(group_name_c.as_ptr());
                if group.is_null() {
                    anyhow::bail!("No group found with the name '{}'", group_name);
                }

                let gid = (*group).gr_gid;

                self.unix_group_id = Some(gid);

                gid
            })
        })
        .transpose()
    }

    pub fn chown(&mut self, path: &str) -> Result<(), anyhow::Error> {
        if let Some(gid) = self.get_gid()? {
            let meta = fs::metadata(path)?;
            let path_c = CString::new(path)?;
            unsafe {
                libc::chown(path_c.as_ptr(), meta.uid(), gid);
            }

            fs::set_permissions(path, Permissions::from_mode(0o774))?;
        }

        Ok(())
    }
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
