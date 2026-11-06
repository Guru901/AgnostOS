//! Minimal PCI configuration-space access used to find the RTL8139.

use x86_64::instructions::port::Port;

const ADDRESS_PORT: u16 = 0x0cf8;
const DATA_PORT: u16 = 0x0cfc;
const RTL8139_VENDOR: u16 = 0x10ec;
const RTL8139_DEVICE: u16 = 0x8139;

#[derive(Clone, Copy, Debug)]
pub(super) struct Rtl8139Pci {
    pub io_base: u16,
}

unsafe fn read_config(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    let address = 0x8000_0000
        | (u32::from(bus) << 16)
        | (u32::from(device) << 11)
        | (u32::from(function) << 8)
        | (u32::from(offset) & 0xfc);
    let mut address_port = Port::<u32>::new(ADDRESS_PORT);
    let mut data_port = Port::<u32>::new(DATA_PORT);
    // SAFETY: these are PCI configuration mechanism #1 ports, and callers
    // serialize each address/data pair by keeping this scan single threaded.
    unsafe {
        address_port.write(address);
        data_port.read()
    }
}

unsafe fn write_config(bus: u8, device: u8, function: u8, offset: u8, value: u32) {
    let address = 0x8000_0000
        | (u32::from(bus) << 16)
        | (u32::from(device) << 11)
        | (u32::from(function) << 8)
        | (u32::from(offset) & 0xfc);
    let mut address_port = Port::<u32>::new(ADDRESS_PORT);
    let mut data_port = Port::<u32>::new(DATA_PORT);
    // SAFETY: see `read_config`; the caller owns device initialization.
    unsafe {
        address_port.write(address);
        data_port.write(value);
    }
}

/// Finds the first RTL8139 and enables I/O port access and PCI bus mastering.
pub(super) fn find_rtl8139() -> Option<Rtl8139Pci> {
    #[cfg(not(target_arch = "x86_64"))]
    return None;

    #[cfg(target_arch = "x86_64")]
    unsafe {
        for bus in 0..=u8::MAX {
            for device in 0..32u8 {
                let header = read_config(bus, device, 0, 0x0c);
                if (header >> 16) as u8 == 0xff {
                    continue;
                }
                let function_count = if (header >> 23) & 1 != 0 { 8 } else { 1 };
                for function in 0..function_count {
                    let id = read_config(bus, device, function, 0x00);
                    if id as u16 != RTL8139_VENDOR || (id >> 16) as u16 != RTL8139_DEVICE {
                        continue;
                    }
                    let command_status = read_config(bus, device, function, 0x04);
                    let command = (command_status as u16) | 0x0005;
                    // Write only the command half. PCI status bits are
                    // write-one-to-clear and must not be echoed back as ones.
                    write_config(bus, device, function, 0x04, u32::from(command));
                    let bar0 = read_config(bus, device, function, 0x10);
                    if bar0 & 1 == 0 {
                        return None;
                    }
                    let base = (bar0 & !3) as u16;
                    if base == 0 {
                        return None;
                    }
                    return Some(Rtl8139Pci { io_base: base });
                }
            }
        }
        None
    }
}
