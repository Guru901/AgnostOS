//! Kernel startup orchestration.
//!
//! The binary entry point should only translate the UEFI status returned by
//! [`initialize`] into the platform entry convention. Keeping the startup
//! sequence here makes its ordering explicit and gives future subsystems one
//! place to add initialization dependencies.

use crate::kprintln;
use crate::{allocator, console, graphics::Framebuffer, idt, shell, uefi_graphics};
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

    let heap_region = match allocator::initialize_heap() {
        Ok(region) => region,
        Err(error) => fatal("heap initialization failed", error),
    };

    if let Err(error) = allocator::initialize_global(heap_region) {
        fatal("global allocator initialization failed", error);
    }

    idt::init();
    shell::init()
}

fn fatal(message: &str, detail: impl core::fmt::Debug) -> ! {
    kprintln!("{message}: {detail:?}");
    loop {
        x86_64::instructions::hlt();
    }
}
