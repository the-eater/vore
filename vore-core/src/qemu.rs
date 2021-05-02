#![cfg(feature = "host")]

use crate::consts::VORE_CONFIG;
use crate::{GlobalConfig, InstanceConfig};
use anyhow::Context;
use mlua::prelude::LuaError;
use mlua::{
    Function, Lua, LuaSerdeExt, MultiValue, RegistryKey, Table, ToLua, UserData, UserDataMethods,
    Value,
};
use serde::ser::Error;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::{fs, mem};

#[derive(Debug, Default, Deserialize, Clone)]
struct VirtualMachine {
    args: Vec<String>,
    bus_ids: HashMap<String, usize>,
    devices: HashMap<String, String>,
    device: bool,
}

impl UserData for VirtualMachine {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_method_mut("arg", |_, this, args: MultiValue| {
            for item in args.iter() {
                if let Value::String(item) = item {
                    let item = item.to_str()?.to_string();
                    if this.device {
                        let mut items = item.split(',');
                        if let Some(_type) = items.next() {
                            for item in items {
                                if let Some(id) = item.strip_prefix("id=") {
                                    this.devices.insert(_type.to_string(), id.to_string());
                                    break;
                                }
                            }
                        }

                        this.device = false;
                    }

                    if item == "-device" {
                        this.device = true;
                    }

                    this.args.push(item)
                }
            }

            Ok(Value::Nil)
        });

        methods.add_method("get_device_id", |lua, this, _type: String| {
            this.devices
                .get(&_type)
                .map_or(Ok(Value::Nil), |x| x.as_str().to_lua(lua))
        });

        methods.add_method_mut("get_next_bus", |lua, this, name: String| {
            let id = this
                .bus_ids
                .entry(name.clone())
                .and_modify(|x| *x += 1)
                .or_insert(0);

            format!("{}.{}", name, id).to_lua(lua)
        });

        methods.add_method_mut("get_counter", |lua, this, args: (String, usize)| {
            let (name, start) = args;

            this.bus_ids
                .entry(name)
                .and_modify(|x| *x += 1)
                .or_insert(start)
                .to_lua(lua)
        });
    }
}

#[derive(Clone, Debug)]
pub struct VoreLuaStorage(Arc<Mutex<VoreLuaStorageInner>>);

impl VoreLuaStorage {
    pub fn weak(&self) -> VoreLuaWeakStorage {
        VoreLuaWeakStorage(Arc::downgrade(&self.0))
    }
}

#[derive(Clone, Debug)]
pub struct VoreLuaWeakStorage(Weak<Mutex<VoreLuaStorageInner>>);

#[derive(Debug)]
pub struct VoreLuaStorageInner {
    build_command: Option<RegistryKey>,
    disk_presets: HashMap<String, VoreLuaDiskPreset>,
    working_dir: PathBuf,
}

#[derive(Debug)]
pub struct VoreLuaDiskPreset {
    description: String,
    callback: RegistryKey,
}

impl UserData for VoreLuaWeakStorage {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_method("set_build_command", |l, weak, func: Function| {
            let strong = weak
                .0
                .upgrade()
                .ok_or_else(|| LuaError::custom("vore storage has expired"))?;
            let mut this = strong
                .try_lock()
                .map_err(|_| LuaError::custom("Failed to lock vore storage"))?;

            if let Some(reg) = this.build_command.take() {
                l.remove_registry_value(reg)?;
            }

            this.build_command = Some(l.create_registry_value(func)?);
            Ok(Value::Nil)
        });

        methods.add_method(
            "register_disk_preset",
            |lua, weak, args: (mlua::String, mlua::String, Function)| {
                let strong = weak
                    .0
                    .upgrade()
                    .ok_or_else(|| LuaError::custom("vore storage has expired"))?;
                let mut this = strong
                    .try_lock()
                    .map_err(|_| LuaError::custom("Failed to lock vore storage"))?;
                let key = lua.create_registry_value(args.2)?;

                let new_preset = VoreLuaDiskPreset {
                    description: args.1.to_str()?.to_string(),
                    callback: key,
                };

                if let Some(old) = this
                    .disk_presets
                    .insert(args.0.to_str()?.to_string(), new_preset)
                {
                    lua.remove_registry_value(old.callback)?;
                }

                Ok(Value::Nil)
            },
        );

        methods.add_method("get_file", |lua, weak, args: (String, String)| {
            let (target, source) = args;
            let strong = weak
                .0
                .upgrade()
                .ok_or_else(|| LuaError::custom("vore storage has expired"))?;
            let this = strong
                .try_lock()
                .map_err(|_| LuaError::custom("Failed to lock vore storage"))?;

            let target = this.working_dir.join(target);
            if !target.exists() {
                if let Some(parent) = target.parent() {
                    if !parent.is_file() {
                        std::fs::create_dir_all(parent)?;
                    }
                }
                std::fs::copy(source, &target)?;
            }

            let path_str = target
                .to_str()
                .ok_or_else(|| LuaError::custom("Path can't be made into string"))?;
            path_str.to_lua(lua)
        });

        methods.add_method(
            "add_disk",
            |lua,
             weak,
             args: (VirtualMachine, mlua::Table, u64, mlua::Table)|
             -> Result<Value, mlua::Error> {
                let (vm, instance, index, disk): (VirtualMachine, mlua::Table, u64, Table) = args;
                let function = {
                    let strong = weak
                        .0
                        .upgrade()
                        .ok_or_else(|| LuaError::custom("vore storage has expired"))?;
                    let this = strong
                        .try_lock()
                        .map_err(|_| LuaError::custom("Failed to lock vore storage"))?;

                    let preset_name = disk
                        .get::<&str, String>("preset")
                        .with_context(|| format!("Disk {} has no preset", index))
                        .map_err(LuaError::external)?;

                    let preset = this
                        .disk_presets
                        .get(&preset_name)
                        .clone()
                        .with_context(|| {
                            format!("No disk preset with the name '{}' found", preset_name)
                        })
                        .map_err(LuaError::external)?;

                    lua.registry_value::<Function>(&preset.callback)?
                };

                function.call((vm, instance, index, disk))
            },
        )
    }
}

impl VoreLuaStorage {
    pub fn new(working_dir: PathBuf) -> VoreLuaStorage {
        VoreLuaStorage(Arc::new(Mutex::new(VoreLuaStorageInner {
            build_command: None,
            disk_presets: Default::default(),
            working_dir,
        })))
    }
}

pub struct QemuCommandBuilder {
    lua: Lua,
    script: String,
    storage: VoreLuaStorage,
}

impl QemuCommandBuilder {
    pub fn new(
        global: &GlobalConfig,
        working_dir: PathBuf,
    ) -> Result<QemuCommandBuilder, anyhow::Error> {
        let lua = Path::new(VORE_CONFIG)
            .parent()
            .unwrap()
            .join(&global.qemu.script);

        let builder = QemuCommandBuilder {
            lua: Lua::new(),
            script: fs::read_to_string(&lua).with_context(|| {
                format!("Failed to load lua qemu command build script ({:?})", lua)
            })?,
            storage: VoreLuaStorage::new(working_dir),
        };

        builder.init(global)?;
        Ok(builder)
    }

    fn init(&self, global: &GlobalConfig) -> Result<(), anyhow::Error> {
        let globals = self.lua.globals();

        globals.set(
            "tojson",
            self.lua.create_function(|lua, value: Value| {
                let x = serde_json::to_string(&value)
                    .context("Failed transforming value into JSON")
                    .map_err(LuaError::external)?;
                lua.create_string(&x)
            })?,
        )?;

        globals.set("vore", self.storage.weak())?;
        globals.set("global", self.lua.to_value(global)?)?;

        Ok(())
    }

    pub fn list_presets(self) -> anyhow::Result<Vec<(String, String)>> {
        self.lua
            .load(&self.script)
            .eval::<()>()
            .context("Failed to run the configured qemu lua script")?;

        let result = {
            self.storage
                .0
                .lock()
                .unwrap()
                .disk_presets
                .iter()
                .map(|(name, preset)| (name.clone(), preset.description.clone()))
                .collect::<Vec<_>>()
        };

        self.clean_up()?;

        Ok(result)
    }

    pub fn build(self, config: &InstanceConfig) -> Result<Vec<String>, anyhow::Error> {
        self.lua
            .load(&self.script)
            .eval::<()>()
            .context("Failed to run the configured qemu lua script")?;

        let item = VirtualMachine::default();
        let multi = MultiValue::from_vec(vec![self.lua.to_value(config)?, item.to_lua(&self.lua)?]);

        let working_dir = { self.storage.0.lock().unwrap().working_dir.clone() };

        let build_command = if let Some(build_command) = &self
            .storage
            .0
            .lock()
            .map_err(|_| LuaError::custom("Failed to lock vore storage"))?
            .build_command
        {
            self.lua.registry_value::<Function>(build_command)?
        } else {
            anyhow::bail!("No qemu build command registered in lua script");
        };

        let mut vm_instance = build_command.call::<MultiValue, VirtualMachine>(multi)?;

        mem::drop(build_command);

        // Weird building way is for clarity sake
        let mut cmd: Vec<String> = vec![
            "-name".into(),
            format!("guest={},debug-threads=on", config.name),
            // Don't start the machine
            "-S".into(),
            // Set timestamps on log
            "-msg".into(),
            "timestamp=on".into(),
            // Drop privileges as soon as possible
            "-runas".into(),
            "nobody".into(),
        ];

        let working_dir = working_dir
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("Can't change working directory into string"))?;

        // Control socket
        cmd.push("-chardev".to_string());
        cmd.push(format!(
            "socket,id=charmonitor,path={}/qemu.sock,server=on,wait=off",
            working_dir
        ));

        // Set mode to control so we use qapi/qmp instead of readline mode
        cmd.push("-mon".to_string());
        cmd.push("chardev=charmonitor,id=monitor,mode=control".to_string());

        cmd.append(&mut vm_instance.args);

        self.clean_up()?;

        Ok(cmd)
    }

    pub fn clean_up(self) -> anyhow::Result<()> {
        self.lua.globals().raw_remove("vore")?;

        self.lua.gc_collect()?;

        if Arc::strong_count(&self.storage.0) > 1 {
            anyhow::bail!("Something still owns vore, can't continue");
        }

        let x = Arc::try_unwrap(self.storage.0)
            .map_err(|_| anyhow::anyhow!("Something still owns vore, can't continue"))?;
        let storage: VoreLuaStorageInner = x
            .into_inner()
            .map_err(|_| anyhow::anyhow!("Something still owns vore, can't continue"))?;

        self.lua
            .remove_registry_value(storage.build_command.unwrap())?;
        for (_, item) in storage.disk_presets.into_iter() {
            self.lua.remove_registry_value(item.callback)?;
        }

        self.lua.gc_collect()?;

        Ok(())
    }
}
