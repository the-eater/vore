---@param instance Instance
---@return boolean
function is_q35(instance)
  return (instance.chipset == "q35" or string.find(instance.chipset, "pc-q35") == 0)
end

---@param instance Instance
---@param vm VM
---@return VM, string
function ensure_pci(instance, vm)
  if is_q35(instance) then
    local i82801b11 = vm:get_device_id("i82801b11-bridge")
    if i82801b11 == nil then
      i82801b11 = "i82801b11"
      vm:arg("-device", "i82801b11-bridge,id=" .. i82801b11 .. ",bus=" .. vm:get_next_bus("pcie"))
    end

    local pci_bridge = vm:get_device_id("pci-bridge")
    if pci_bridge == nil then
      pci_bridge = "pci-bridge"
      vm:arg("-device", "pci-bridge,chassis_nr=" .. vm:get_counter("chassis", 1) .. ",id=" .. pci_bridge .. ",bus=" .. i82801b11)
    end

    return vm, pci_bridge
  else
    error("No support for non-q35 instances")
  end
end

---@param instance Instance
---@param vm VM
---@param mem_path string
---@param size number
---@param id string
---@return VM
function add_shared_memory(instance, vm, mem_path, size, id)
  local pci
  vm, pci = ensure_pci(instance, vm)

  vm:arg("-object", "memory-backend-file,id=shmem-" .. id .. ",mem-path=" .. mem_path .. ",size=" .. size .. ",share=on")
  vm:arg("-device", "ivshmem-plain,memdev=shmem-" .. id .. ",bus=" .. pci .. ",addr=0x" .. string.format("%x", vm:get_counter("pci", 1)))

  return vm
end

vore:set_build_command(function(instance, vm)
  vm:arg("-rtc", "driftfix=slew")
  --vm:arg("-mon", "stdio")
  vm:arg("-no-hpet")
  vm:arg("-boot", "strict=on")

  vm:arg("-chardev", "socket,id=charmonitor,path=/tmp/qemu.sock,server=on,wait=off")
  vm:arg("-mon", "chardev=charmonitor,id=monitor,mode=readline")

  if instance.kvm then
    vm:arg("-enable-kvm")
  end

  if instance.arch == "x86_64" or instance.arch == "i868" then
    vm:arg("-global", "kvm-pit.lost_tick_policy=discard")
  end

  --this disables the QEMU GUI
  --vm:arg("-display", "none")

  vm:arg("-no-user-config")
  --vm:arg("-nodefaults")
  vm:arg("-no-shutdown")
  vm:arg("-m", tostring(instance.memory))

  local cpu = instance.cpu;
  vm:arg(
    "-smp",
    string.format(
      "%d,sockets=%d,dies=%d,cores=%d,threads=%d",
      cpu.amount,
      cpu.sockets,
      cpu.dies,
      cpu.cores,
      cpu.threads
    )
  )

  if instance.uefi.enabled and is_q35(instance) then
    -- OVMF will hang if S3 is not disabled
    -- disable S4 too, since libvirt does that 🤷
    -- https://bugs.archlinux.org/task/59465#comment172528
    vm:arg("-global", "ICH9-LPC.disable_s3=1")
    vm:arg("-global", "ICH9-LPC.disable_s4=1")
  end

  for idx, disk in ipairs(instance.disks) do
    vm = vore:add_disk(vm, idx, disk)
  end

  if instance.uefi.enabled then
    vm:arg(
      "-drive", "if=pflash,format=raw,unit=0,file=" .. global.uefi.default.boot_code .. ",readonly=on",
      "-drive", "if=pflash,format=raw,unit=1,file=" .. vore:get_file("uefi/OVMF_VARS.fd", global.uefi.default.template)
    )
  end

  for _, vfio in ipairs(instance.vfio) do
    local def = "vfio-pci,host=" .. vfio.slot
    if vfio.graphics then
      def = def .. ",x-vga=on"
    end

    if vfio.multifunction then
      def = def .. ",multifunction=on"
    end

    if vfio.graphics and vm:get_counter("disabled_display", 0) == 0 then
      vm:arg("-vga", "none")
    end

    vm:arg("-device", def)
  end

  if instance.looking_glass.enabled then
    vm = add_shared_memory(instance, vm, instance.looking_glass.mem_path, instance.looking_glass.buffer_size, "lg")
  end

  if instance.scream.enabled then
    vm = add_shared_memory(instance, vm, instance.scream.mem_path, instance.scream.buffer_size, "scream")
  end

  if instance.spice.enabled then
    vm:arg("-spice", "unix,addr=" .. instance.spice.socket_path .. ",disable-ticketing=on,seamless-migration=on")
  end

  vm:arg(
    "-machine",
    "q35,accel=kvm,usb=off,vmport=off,dump-guest-core=off,kernel_irqchip=on"
  )

  vm:arg(
    "-cpu",
    "host,migratable=on,hv-time,hv-relaxed,hv-vapic,hv-spinlocks=0x1fff,hv-vendor-id=whatever,kvm=off"
  )

  return vm
end)

---
---@param type string
---@return fun(vm: VM, idx: number, disk: Disk): VM
function scsi_disk_gen(type)
  return function(vm, idx, disk)
    vm:arg(
      "-blockdev",
      tojson({
        ["driver"] = "raw",
        ["file"] = {
          ["driver"] = "host_device",
          ["filename"] = disk.path,
          ["aio"] = "native",
          ["discard"] = "unmap",
          ["cache"] = { ["direct"] = true, ["no-flush"] = false },
        },
        ["node-name"] = "format-" .. idx,
        ["read-only"] = false,
        ["cache"] = { ["direct"] = true, ["no-flush"] = false },
        ["discard"] = "unmap",
      })
    )

    local scsi_pci = vm:get_device_id("virtio-scsi-pci")
    if scsi_pci == nil then
      scsi_pci = "scsi-pci"
      vm:arg("-device", "virtio-scsi-pci,id=" .. scsi_pci)
    end

    local hd = "scsi-hd,drive=format-" .. idx .. ",bus=" .. scsi_pci .. ".0"
    if type == "ssd" then
      -- Having a rotation rate of 1 signals Windows it's an ssd
      hd = hd .. ",rotation_rate=1"
    end

    vm:arg("-device", hd)

    return vm
  end
end

vore:register_disk_preset("ssd", scsi_disk_gen("ssd"))
vore:register_disk_preset("hdd", scsi_disk_gen("hdd"))