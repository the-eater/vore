use vore_core::InstanceConfig;

fn main() {
    let cfg = InstanceConfig::from_toml(include_str!("../../config/example.toml")).unwrap();
}
