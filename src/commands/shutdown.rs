use x86_64::instructions::port::Port;

#[derive(Clone, Copy)]
pub(crate) struct QemuExitCode(u32);

impl QemuExitCode {
    pub(crate) const SUCCESS: Self = Self(0);
}

pub(crate) fn exit_qemu(code: QemuExitCode) -> ! {
    // SAFETY: port `0xf4` is the QEMU isa-debug-exit device configured for this
    // kernel. Calling this on other hardware can have device-specific effects.
    unsafe {
        let mut port = Port::<u32>::new(0xf4);
        port.write(code.0);
    }

    loop {
        x86_64::instructions::hlt();
    }
}
