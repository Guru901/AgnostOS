#![no_main]
#![no_std]

extern crate alloc;

use agnostos::{
    allocator, boot_services_exited, console, graphics::Framebuffer, idt, kprintln, shell,
    timer::sleep_ms, uefi_graphics,
};

#[cfg(feature = "custom-allocator")]
use agnostos::allocator::AgnostOSAllocator;

#[cfg(not(feature = "custom-allocator"))]
use linked_list_allocator::LockedHeap;

use uefi::prelude::*;

#[cfg(feature = "custom-allocator")]
#[global_allocator]
pub static ALLOCATOR: AgnostOSAllocator = AgnostOSAllocator::new();

#[cfg(not(feature = "custom-allocator"))]
#[global_allocator]
pub static ALLOCATOR: LockedHeap = LockedHeap::empty();

#[entry]
fn main() -> Status {
    if let Err(error) = uefi::helpers::init() {
        return error.status();
    }

    let mut gop = match uefi_graphics::init_gop() {
        Ok(gop) => gop,
        Err(error) => return error.status(),
    };
    let fb = match Framebuffer::new(&mut gop) {
        Ok(fb) => fb,
        Err(error) => {
            uefi::println!("Unsupported framebuffer configuration: {error:?}");
            return Status::UNSUPPORTED;
        }
    };

    console::init(fb);
    uefi::println!("Exiting boot services in 1 seconds...");

    let heap_region = match allocator::initialize_heap() {
        Ok(region) => region,
        Err(error) => fatal_after_boot("heap initialization failed", error),
    };

    #[cfg(feature = "custom-allocator")]
    if let Err(error) = ALLOCATOR.init(heap_region) {
        fatal_after_boot("custom allocator initialization failed", error);
    }

    #[cfg(not(feature = "custom-allocator"))]
    allocator::initialize_linked_list_allocator(&ALLOCATOR, heap_region);

    idt::init();
    x86_64::instructions::interrupts::enable();

    shell::init()
}

fn fatal_after_boot(_message: &str, _detail: impl core::fmt::Debug) -> ! {
    loop {
        x86_64::instructions::hlt();
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    if boot_services_exited() {
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
        x86_64::instructions::hlt();
    }
}
