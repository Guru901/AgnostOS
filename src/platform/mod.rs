//! Target-specific CPU operations with safe host-test fallbacks.

pub(crate) mod ring_buffer;

/// Runs `operation` with interrupts disabled on the x86_64 kernel target.
/// Host tests have no hardware interrupts, so they simply run the operation.
#[inline]
pub(crate) fn without_interrupts<R>(operation: impl FnOnce() -> R) -> R {
    #[cfg(target_arch = "x86_64")]
    {
        x86_64::instructions::interrupts::without_interrupts(operation)
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        operation()
    }
}

/// Reports whether CPU interrupts are enabled on the kernel target.
#[inline]
pub(crate) fn interrupts_enabled() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        x86_64::instructions::interrupts::are_enabled()
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

/// Halts the CPU on the kernel target; yields the processor in host builds.
#[inline]
pub(crate) fn halt() {
    #[cfg(target_arch = "x86_64")]
    {
        x86_64::instructions::hlt();
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        core::hint::spin_loop();
    }
}

/// Writes a 32-bit value to an I/O port on x86_64. Host builds do nothing.
///
/// # Safety
///
/// `port` and `value` must be valid for the active device protocol.
#[inline]
pub(crate) unsafe fn write_port_u32(port: u16, value: u32) {
    #[cfg(target_arch = "x86_64")]
    {
        use x86_64::instructions::port::Port;

        let mut port = Port::<u32>::new(port);
        // SAFETY: upheld by this function's caller.
        unsafe { port.write(value) };
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = (port, value);
    }
}
