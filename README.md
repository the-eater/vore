# vore

> VFIO Orientated Emulation (_definitely_)

## What is vore?

`vore` is a virtual machine management tool focused on VFIO set ups. with a minimal TOML file you should be able to get
you should be able to create a VFIO-focused VM.

It features close integration for this use cases, for example automatic configuration of Looking Glass.

## How it works

`vore` loads a TOML file, sends it to the `vored` daemon, which processes it and auto completes required information,
and then passes it to a Lua script. this Lua script builds up the qemu command, which then gets started and managed
by `vored`.

`vored` also allows you to save definitions, and `reserve` vfio devices, so that they are claimed at system start up.

## Requirements

Building:

- Rust
- Lua 5.4 (including headers)

Runtime:

- Lua 5.4

## VM Definition

This is a annotated VM definition with about every option displayed

```toml
[machine]
# Name of the VM, this will be the name used internally and externally for the vm
name = "win10"
# Amount of memory for the virtual machine
memory = "12G"
# Shorthand for <feature>.enabled = true
features = [
    "uefi",
    "spice",
    "pulse",
    "looking-glass"
]
# If vore should automatically start this VM when the daemon starts
#auto-start = false

[cpu]
# Amount of vCPU's should be given to the 
amount = 12
# If any of the following are given, vore will automatically calculate
# the amount of vCPU's, however if both are given, vore will verify it's correctness
# Amount of threads ever core has
# If amount is even or not set, this is set to 2, if odd, it's set to 1
#threads = 2
# Amount of cores on this die
# If amount is even, this is set to amount/2, if odd it's set to amount
# If amount is not set this is 2
#cores = 6
# Amount of dies per socket, defaults to 1
#dies = 1
# Amount of sockets, defaults to 1
#sockets = 1

# You can add multiple disks by adding more `[[disk]]` entries
[[disk]]
# Preset used for this disk, defined in qemu.lua, 
# run `vore disk preset` to list all available presets
preset = "nvme"
# Path to disk file  
path = "/dev/disk/by-id/nvme-eui.6479a74530201073"
# Type of disk file, will be automatically set, 
# but vore will tell you if it can't figure it out
#disk_type = "raw"

[[vfio]]
# If when this VM is saved, vored should try to automatically 
# bind it to the vfio-pci driver
reserve = true
# vendor, device and index (0-indexed!) can be used to select a certain card
# this will grab the second GTX 1080 in the system
vendor = 0x10de
device = 0x1b80
index = 1
# you can also instead set addr directly
# -however- if you set both vore will check if both match and error out if not
# this can be helpful when passing through system devices,
# which may move after insertion of e.g. nvme drive
addr = "0b:00.3"

# if this device is a graphics card
# it'll both set x-vga, and disable QEMU's virtual GPU
graphics = true

# if this device is multifunctional
#multifunction = false

[pulse]
# If a pulseaudio backed audio device should be created
# using the features shorthand is preferred
#enabled = true
# Path to PulseAudio native socket 
# if not specified vore will automatically resolve it
#socket-path = ""
# To which user's PulseAudio session it should connect
# Can be prefixed with # to set an id
# Default is #1000, which is the common default user id
#user = "#1000"

[spice]
# if spice support should be enabled
# using the features shorthand is preferred
#enabled = true
# on which path the SPICE socket should listen
# If not set vore will use /var/lib/vore/instance/<name>/spice.sock
#socket-path = "/run/spicy.sock"

[looking-glass]
# if looking-glass support should be enabled
# using the features shorthand is preferred
#enabled = true
# width, height, and bit depth of the screen LG will transfer 
# this info is used to calculate the required shared memory file size 
width = 2560
height = 1080
#bit-depth = 8
# Alternatively you can set the buffer size directly
# vore will automatically pick the lowest higher or equal to buffer-size
# that is a power of 2
#buffer-size = 999999
# Path to the shared memory file looking-glass should use
# if not specified vore will create a path.
# this is mostly for in the case you use the kvmfr kernel module
#mem-path = "/dev/kvmfr0" 
```


# TODO

- [ ] hugepages support
- [ ] USB passthrough
- [ ] Hot-plug USB via `vore attach <addr>`
- [ ] jack audiodev support
- [ ] qemu cmdline on request (`vore x qemucmd`)
- [ ] Better CPU support and feature assignment
- [ ] more control over CPU pinning (now just pickes the fist amount of CPU's)
- [ ] Network device configuration