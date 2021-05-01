// File with all static constants like e.g. paths
#![allow(clippy::manual_unwrap_or)]

macro_rules! default_env {
    ($val:expr, $def:expr) => {
        match option_env!($val) {
            None => $def,
            Some(x) => x,
        };
    };
}

pub const VORE_DIRECTORY: &str = default_env!("VORE_DIRECTORY", "/var/lib/vore");
pub const VORE_SOCKET: &str = default_env!("VORE_SOCKET", "/run/vore.sock");
#[cfg(debug_assertions)]
pub const VORE_CONFIG: &str = default_env!(
    "VORE_CONFIG",
    concat!(file!(), "/../../../../config/vored.toml")
);
#[cfg(not(debug_assertions))]
pub const VORE_CONFIG: &str = default_env!("VORE_CONFIG", "/etc/vore/vored.toml");
