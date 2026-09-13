//! Kernel startup orchestration.
//!
//! The binary entry point should only translate the UEFI status returned by
//! [`initialize`] into the platform entry convention. Keeping the startup
//! sequence here makes its ordering explicit and gives future subsystems one
//! place to add initialization dependencies.

use crate::kprintln;
#[cfg(not(feature = "fault-smoke"))]
use crate::shell;
use crate::{allocator, console, graphics::Framebuffer, interrupts, paging, uefi_graphics};
use uefi::Status;
use uefi::{boot, proto::loaded_image::LoadedImage};

/// Initializes the UEFI-facing parts of the kernel and enters the shell.
pub fn initialize() -> Status {
    if let Err(error) = uefi::helpers::init() {
        return error.status();
    }

    let loaded_image = match boot::open_protocol_exclusive::<LoadedImage>(boot::image_handle()) {
        Ok(image) => image,
        Err(error) => return error.status(),
    };
    let (image_base, image_size) = loaded_image.info();
    let Some(image_size) = usize::try_from(image_size).ok() else {
        return Status::BAD_BUFFER_SIZE;
    };
    let kernel_image = (image_base as usize, image_size);
    drop(loaded_image);

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

    let heap_region =
        match allocator::initialize_heap(Some(framebuffer.physical_range()), kernel_image) {
            Ok(region) => region,
            Err(error) => fatal("heap initialization failed", error),
        };

    if let Err(error) = allocator::initialize_global(heap_region) {
        fatal("global allocator initialization failed", error);
    }

    if let Err(error) = paging::initialize(framebuffer.physical_range(), kernel_image) {
        fatal("page-table initialization failed", error);
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
