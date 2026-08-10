#![no_main]
#![no_std]

extern crate alloc;

use agnostos::{
    allocator, boot_services_exited, console, graphics::Framebuffer, idt, kprintln, shell,
    uefi_graphics,
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
    uefi::helpers::init().unwrap();

    let mut gop = uefi_graphics::init_gop();
    let fb = match Framebuffer::new(&mut gop) {
        Ok(fb) => fb,
        Err(error) => {
            uefi::println!("Unsupported framebuffer configuration: {error:?}");
            return Status::UNSUPPORTED;
        }
    };

    console::init(fb);
    uefi::println!("Exiting boot services in 1 seconds...");

    let heap_region = allocator::initialize_heap();

    #[cfg(feature = "custom-allocator")]
    ALLOCATOR.init(heap_region);

    #[cfg(not(feature = "custom-allocator"))]
    allocator::initialize_linked_list_allocator(&ALLOCATOR, heap_region);

    idt::init();
    x86_64::instructions::interrupts::enable();

    shell::init()
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
