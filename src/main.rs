#![no_main]
#![no_std]

extern crate alloc;

use agnostos::{
    BOOT_SERVICES_EXITED, allocator::AgnostOSAllocator, console, graphics::Framebuffer, kprintln,
    shell, uefi_graphics,
};

use uefi::prelude::*;

#[global_allocator]
pub static ALLOCATOR: AgnostOSAllocator = AgnostOSAllocator::new();

#[entry]
fn main() -> Status {
    uefi::helpers::init().unwrap();

    let mut gop = uefi_graphics::init_gop();
    let fb = match Framebuffer::new(&mut gop) {
        Ok(fb) => fb,
        Err(error) => {
            uefi::println!("Unsupported framebuffer configuration: {error:?}");
            return Status::UNSUPPORTED;
        }
    };

    console::init(&fb);
    uefi::println!("Exiting boot services in 1 seconds...");

    ALLOCATOR.init();
    shell::init(&fb)
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    if BOOT_SERVICES_EXITED.load(core::sync::atomic::Ordering::Relaxed) {
        kprintln!("========================================");
        kprintln!("              KERNEL PANIC");
        kprintln!("========================================");
        kprintln!();

        if let Some(location) = info.location() {
            kprintln!(
                "Location : {}:{}:{}",
                location.file(),
                location.line(),
                location.column()
            );
        }

        kprintln!("Message  : {}", info.message());
        kprintln!();
        kprintln!("System halted.");
    } else {
        uefi::println!("========================================");
        uefi::println!("              KERNEL PANIC");
        uefi::println!("========================================");
        uefi::println!("{info}");
    }

    loop {
        core::hint::spin_loop();
    }
}
