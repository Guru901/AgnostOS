//! Kernel startup orchestration.
//!
//! The binary entry point should only translate the UEFI status returned by
//! [`initialize`] into the platform entry convention. Keeping the startup
//! sequence here makes its ordering explicit and gives future subsystems one
//! place to add initialization dependencies.

use crate::kprintln;
#[cfg(not(feature = "fault-smoke"))]
use crate::shell;
use crate::{allocator, console, graphics::Framebuffer, interrupts, uefi_graphics};
use uefi::Status;

/// Initializes the UEFI-facing parts of the kernel and enters the shell.
pub fn initialize() -> Status {
    if let Err(error) = uefi::helpers::init() {
        return error.status();
    }

    let mut gop = match uefi_graphics::init_gop() {
        Ok(gop) => gop,
        Err(error) => return error.status(),
    };

    let framebuffer = match Framebuffer::new(&mut gop) {
        Ok(framebuffer) => framebuffer,
        Err(error) => {
            uefi::println!("Unsupported framebuffer configuration: {error:?}");
            return Status::UNSUPPORTED;
        }
    };

    console::init(framebuffer);
    uefi::println!("Exiting boot services in 1 seconds...");

    let heap_region = match allocator::initialize_heap(Some(framebuffer.physical_range())) {
        Ok(region) => region,
        Err(error) => fatal("heap initialization failed", error),
    };

    if let Err(error) = allocator::initialize_global(heap_region) {
        fatal("global allocator initialization failed", error);
    }

    interrupts::init();
    #[cfg(feature = "input-smoke")]
    crate::input_smoke::ready();
    interrupts::enable_runtime();
    #[cfg(feature = "fault-smoke")]
    {
        crate::fault_smoke::trigger();
    }
    #[cfg(not(feature = "fault-smoke"))]
    shell::init()
}

fn fatal(message: &str, detail: impl core::fmt::Debug) -> ! {
    kprintln!("{message}: {detail:?}");
    loop {
        crate::platform::halt();
    }
}
