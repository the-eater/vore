mod global_config;
mod instance_config;
mod qemu;
mod virtual_machine;
mod cpu_list;
pub mod rpc;
pub mod consts;
mod virtual_machine_info;

pub use global_config::*;
pub use instance_config::*;
pub use qemu::QemuCommandBuilder;
#[cfg(feature = "host")]
pub use virtual_machine::*;
pub use virtual_machine_info::*;

pub fn init_logging() {
    let mut builder = pretty_env_logger::formatted_timed_builder();
    #[cfg(debug_assertions)] {
        use log::LevelFilter;
        builder.filter_level(LevelFilter::Debug);
    }
    builder.parse_filters(&std::env::var("RUST_LOG").unwrap_or_else(|_| "".to_string()));
    builder.init();
}