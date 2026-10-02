//! UEFI entry for the AgnostOS kernel.
//!
//! `main` only hands control to [`agnostos::boot::initialize`]. After
//! ExitBootServices, panics print through the kernel console; before that they
//! still use UEFI `println!`.
#![no_main]
#![no_std]

use agnostos::{boot, boot_services_exited, kprintln};
use uefi::prelude::*;

/// Firmware entry: set up the kernel and never return except as a UEFI status.
#[entry]
fn main() -> Status {
    boot::initialize()
}

/// After ExitBootServices, prints through [`kprintln!`]; earlier panics use UEFI output.
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
