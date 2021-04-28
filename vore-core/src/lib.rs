mod global_config;
mod instance_config;
mod qemu;
mod virtual_machine;
mod cpu_list;
pub mod rpc;

pub use global_config::*;
pub use instance_config::*;
pub use qemu::QemuCommandBuilder;
pub use virtual_machine::*;
use log::LevelFilter;

pub fn init_logging() {
    let mut builder = pretty_env_logger::formatted_timed_builder();
    #[cfg(debug_assertions)] {
        builder.filter_level(LevelFilter::Debug);
    }
    builder.parse_filters(&std::env::var("RUST_LOG").unwrap_or("".to_string()));
    builder.init();
}