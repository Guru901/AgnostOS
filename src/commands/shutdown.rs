use crate::platform;

#[derive(Clone, Copy)]
pub(crate) struct QemuExitCode(u32);

impl QemuExitCode {
    pub(crate) const SUCCESS: Self = Self(0);
}

pub(crate) fn exit_qemu(code: QemuExitCode) -> ! {
    // SAFETY: port `0xf4` is the QEMU isa-debug-exit device configured for this
    // kernel. Calling this on other hardware can have device-specific effects.
    unsafe {
        platform::write_port_u32(0xf4, code.0);
    }

    loop {
        platform::halt();
    }
}
