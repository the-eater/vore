function build_command(instance, args)
    args:add("-rtc", "driftfix=slew")
    args:add("-serial", "stdio")
    args:add("-no-hpet")
    args:add("-boot", "strict=on")

    if instance.kvm then
        args:add("-enable-kvm")
    end

    if instance.arch == "x86_64" or instance.arch == "i868" then
        args:add("-global", "kvm-pit.lost_tick_policy=discard")
    end

    args:add("-no-user-config")
    args:add("-no-defaults")
    args:add("-no-shutdown")
    args:add("-m", tostring(instance.memory))

    local cpu = instance.cpu;
    args:add(string.format("%d,sockets=%d,dies=%d,cores=%d,threads=%d",
            cpu.amount,
            cpu.sockets,
            cpu.dies,
            cpu.cores,
            cpu.threads))

    if instance.uefi.enabled and string.find(instance.chipset, "q35") == 0 then
        -- OVMF will hang if S3 is not disabled
        -- disable S4 too, since libvirt does that 🤷
        -- https://bugs.archlinux.org/task/59465#comment172528
        args:add("-global", "ICH9-LPC.disable_s3=1")
        args:add("-global", "ICH9-LPC.disable_s4=1")
    end

    return args
end