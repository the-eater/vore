--- Global configuration

---@class GlobalUefi
---@field boot_code string
---@field template string

---@class global
---@field uefi table<string, GlobalUefi>
global = {}

---@class Vore
vore = {}

---@class VM
VM = {}

----
---Encodes input as json (implemented in Rust)
---@param input any
---@return string
function tojson(input)
end

---Add an argument to the current argument list for this vm
---@vararg string
function VM:arg(...)
end

---Get the next available bus for a post
---@param port string
---@return string
function VM:get_next_bus(port)
end

---Get a new counter that +1 every cal
---@field name string
---@field default number
---@return number
function VM:get_counter(name, default)
end

---Get the last device id of a added device
---@param device_name string
---@return string
function VM:get_device_id(device_name)
end

---@class Disk
---@field preset string
---@field disk_type string
---@field path string


---@class Cpu
---@field amount number
---@field sockets number
---@field dies number
---@field cores number
---@field threads number

---@class Uefi
---@field enabled boolean

---@class LookingGlass
---@field enabled boolean
---@field mem_path string
---@field buffer_size number

---@class Scream
---@field enabled boolean
---@field mem_path string
---@field buffer_size number

---@class Vfio
---@field slot string
---@field graphics boolean
---@field multifunction boolean

---@class Spice
---@field enabled boolean
---@field socket_path string

---@class Instance
---@field name string
---@field kvm boolean
---@field arch string
---@field memory number
---@field chipset string
---@field disks Disk[]
---@field cpu Cpu
---@field uefi Uefi
---@field vfio Vfio[]
---@field looking_glass LookingGlass
---@field scream Scream
---@field spice Spice

----
---Add a disk definition to the argument list
---@param vm VM
---@param index number
---@param disk Disk
---@return VM
function vore:add_disk(vm, index, disk)
end

----
---Register a disk preset
---@param name string
---@param cb fun(vm: VM, idx: number, disk: Disk): VM
function vore:register_disk_preset(name, cb)
end

---set_build_command
---@param cb fun(instance: Instance, vm: VM)
function vore:set_build_command(cb)
end

---Get a local file based from a template
---If the target file doesn't exist yet it will be created from the source file
---@param target string The target path within the local working directory
---@param source_file string the source or template file
function vore:get_file(target, source_file)
end