use crate::{GlobalConfig, InstanceConfig};
use mlua::{Function, LuaSerdeExt, MultiValue, ToLua, UserData, UserDataMethods, Value};
use serde::Deserialize;

#[derive(Debug, Default, Deserialize, Clone)]
struct LuaFreeList(Vec<String>);

impl UserData for LuaFreeList {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_method_mut("add", |_, this, args: MultiValue| {
            for item in args.iter() {
                if let Value::String(item) = item {
                    this.0.push(item.to_str()?.to_string())
                }
            }

            Ok(Value::Nil)
        })
    }
}

pub fn build_qemu_command(config: &InstanceConfig, global_config: &GlobalConfig) -> Vec<String> {
    let lua = mlua::Lua::new();
    // TODO: load correct script
    lua.load(include_str!("../../config/qemu.lua"))
        .eval::<()>()
        .unwrap();
    let val: Function = lua.globals().get("build_command").unwrap();
    let item = LuaFreeList::default();
    let multi = MultiValue::from_vec(vec![
        lua.to_value(config).unwrap(),
        item.to_lua(&lua).unwrap(),
    ]);
    let mut x = val.call::<MultiValue, LuaFreeList>(multi).unwrap();
    println!("{:?}", x);

    let mut cmd: Vec<String> = vec![];
    cmd.push("-name".to_string());
    cmd.push(format!("guest={},debug-threads=on", config.name));

    cmd.push("-S".to_string());
    cmd.push("-msg".to_string());
    cmd.push("timestamps=on".to_string());
    cmd.append(&mut x.0);

    cmd
}
