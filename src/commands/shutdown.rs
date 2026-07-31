use x86_64::instructions::port::Port;

pub fn exit_qemu(code: u32) -> ! {
    unsafe {
        let mut port = Port::<u32>::new(0xf4);
        port.write(code);
    }

    loop {
        x86_64::instructions::hlt();
    }
}
