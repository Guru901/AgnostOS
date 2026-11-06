//! Polling driver for the QEMU-compatible RTL8139 PCI Ethernet controller.

use core::{
    ptr,
    sync::atomic::{AtomicBool, Ordering},
};
use spin::Mutex;
use x86_64::instructions::port::Port;

use super::{MacAddress, PacketError, pci};

const RX_RING_LEN: usize = 8192;
const RX_BUFFER_LEN: usize = RX_RING_LEN + 16 + 2048;
const TX_BUFFER_LEN: usize = 2048;
const ETH_FRAME_MAX: usize = 1518;
const RX_OK: u16 = 1 << 0;
const RX_ERROR: u16 = (1 << 1) | (1 << 2) | (1 << 3) | (1 << 4);
const TX_OK: u32 = 1 << 15;
const TX_ABORTED: u32 = 1 << 30;

#[repr(align(256))]
struct RxBuffer([u8; RX_BUFFER_LEN]);

#[repr(align(16))]
struct TxBuffer([u8; TX_BUFFER_LEN]);

static mut RX_BUFFER: RxBuffer = RxBuffer([0; RX_BUFFER_LEN]);
static mut TX_BUFFERS: [TxBuffer; 4] = [const { TxBuffer([0; TX_BUFFER_LEN]) }; 4];
static INITIALIZED: AtomicBool = AtomicBool::new(false);
static DRIVER: Mutex<Option<Driver>> = Mutex::new(None);

struct Driver {
    io_base: u16,
    mac: MacAddress,
    rx_offset: usize,
    tx_index: usize,
}

impl Driver {
    fn port(&self, offset: u16) -> u16 {
        self.io_base + offset
    }

    unsafe fn read8(&self, offset: u16) -> u8 {
        let mut port = Port::<u8>::new(self.port(offset));
        // SAFETY: caller must use a valid RTL8139 register offset.
        unsafe { port.read() }
    }

    unsafe fn read32(&self, offset: u16) -> u32 {
        let mut port = Port::<u32>::new(self.port(offset));
        // SAFETY: caller must use a valid RTL8139 register offset.
        unsafe { port.read() }
    }

    unsafe fn write8(&self, offset: u16, value: u8) {
        let mut port = Port::<u8>::new(self.port(offset));
        // SAFETY: caller must use a valid RTL8139 register offset/value.
        unsafe { port.write(value) }
    }

    unsafe fn write16(&self, offset: u16, value: u16) {
        let mut port = Port::<u16>::new(self.port(offset));
        // SAFETY: caller must use a valid RTL8139 register offset/value.
        unsafe { port.write(value) }
    }

    unsafe fn write32(&self, offset: u16, value: u32) {
        let mut port = Port::<u32>::new(self.port(offset));
        // SAFETY: caller must use a valid RTL8139 register offset/value.
        unsafe { port.write(value) }
    }
}

/// Initializes the controller in polling mode. Safe to call once during boot.
pub(super) fn initialize() -> Result<MacAddress, DriverError> {
    if INITIALIZED.load(Ordering::Acquire) {
        return DRIVER
            .lock()
            .as_ref()
            .map(|driver| driver.mac)
            .ok_or(DriverError::NotFound);
    }
    let device = pci::find_rtl8139().ok_or(DriverError::NotFound)?;
    let mut driver = Driver {
        io_base: device.io_base,
        mac: MacAddress::default(),
        rx_offset: 0,
        tx_index: 0,
    };
    let mut mac = [0u8; 6];

    // SAFETY: PCI discovery found this RTL8139's I/O BAR and bus mastering is
    // enabled. The driver is the only code touching its registers.
    unsafe {
        driver.write8(0x52, 0x00); // power on
        driver.write8(0x37, 0x10); // reset
        let mut reset_timeout = 1_000_000usize;
        while driver.read8(0x37) & 0x10 != 0 && reset_timeout != 0 {
            reset_timeout -= 1;
            core::hint::spin_loop();
        }
        if reset_timeout == 0 {
            return Err(DriverError::ResetTimeout);
        }

        for (index, byte) in mac.iter_mut().enumerate() {
            *byte = driver.read8(index as u16);
        }
        driver.mac = MacAddress::new(mac);

        let rx_address = ptr::addr_of_mut!(RX_BUFFER.0) as u64;
        let tx_address = ptr::addr_of_mut!(TX_BUFFERS[0].0) as u64;
        if rx_address > u64::from(u32::MAX) || tx_address > u64::from(u32::MAX) {
            return Err(DriverError::DmaAddressTooHigh);
        }
        driver.write32(0x30, rx_address as u32);
        for index in 0..4 {
            let address = ptr::addr_of_mut!(TX_BUFFERS[index].0) as u64;
            if address > u64::from(u32::MAX) {
                return Err(DriverError::DmaAddressTooHigh);
            }
            driver.write32(0x20 + index as u16 * 4, address as u32);
        }
        driver.write16(0x3c, 0); // disable device IRQs; the driver polls
        driver.write16(0x3e, 0xffff); // clear pending status
        driver.write32(0x44, 0x0000_008f); // 8 KiB ring, accept unicast/broadcast
        driver.write8(0x37, 0x0c); // enable receiver and transmitter
    }

    *DRIVER.lock() = Some(driver);
    INITIALIZED.store(true, Ordering::Release);
    Ok(MacAddress::new(mac))
}

/// Controller setup and packet-path failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DriverError {
    NotFound,
    ResetTimeout,
    DmaAddressTooHigh,
    TransmitTimeout,
    ReceiveCorrupt,
    Packet(PacketError),
}

/// Transmit one Ethernet frame and wait until the controller releases its slot.
pub(super) fn transmit(frame: &[u8]) -> Result<(), DriverError> {
    if frame.len() > TX_BUFFER_LEN || frame.len() < 14 {
        return Err(DriverError::Packet(PacketError::BufferTooSmall));
    }
    let mut guard = DRIVER.lock();
    let driver = guard.as_mut().ok_or(DriverError::NotFound)?;
    let slot = driver.tx_index;
    // SAFETY: the selected DMA slot is exclusively owned until TSD reports a
    // completion. This single-core kernel serializes calls with DRIVER.
    unsafe {
        ptr::copy_nonoverlapping(
            frame.as_ptr(),
            ptr::addr_of_mut!(TX_BUFFERS[slot].0).cast(),
            frame.len(),
        );
        driver.write32(0x10 + slot as u16 * 4, frame.len() as u32);
        let mut timeout = 10_000_000usize;
        loop {
            let status = driver.read32(0x10 + slot as u16 * 4);
            if status & TX_ABORTED != 0 {
                return Err(DriverError::TransmitTimeout);
            }
            if status & TX_OK != 0 {
                break;
            }
            if timeout == 0 {
                return Err(DriverError::TransmitTimeout);
            }
            timeout -= 1;
            core::hint::spin_loop();
        }
    }
    driver.tx_index = (slot + 1) & 3;
    Ok(())
}

/// Copy one pending Ethernet frame into `out`, or return `Ok(None)` if empty.
pub(super) fn receive(out: &mut [u8]) -> Result<Option<usize>, DriverError> {
    let mut guard = DRIVER.lock();
    let driver = guard.as_mut().ok_or(DriverError::NotFound)?;
    // SAFETY: CR.BUFE says the controller has no complete frame in the ring.
    unsafe {
        if driver.read8(0x37) & 1 != 0 {
            return Ok(None);
        }
        let ring = ptr::addr_of!(RX_BUFFER.0).cast::<u8>();
        let offset = driver.rx_offset;
        let status = u16::from_le(ptr::read_unaligned(ring.add(offset).cast::<u16>()));
        let length = usize::from(u16::from_le(ptr::read_unaligned(
            ring.add(offset + 2).cast::<u16>(),
        )));
        if status & RX_ERROR != 0
            || status & RX_OK == 0
            || !(18..=ETH_FRAME_MAX + 4).contains(&length)
        {
            return Err(DriverError::ReceiveCorrupt);
        }
        let frame_len = length - 4; // remove Ethernet CRC
        if out.len() < frame_len {
            return Err(DriverError::Packet(PacketError::BufferTooSmall));
        }
        let data_start = (offset + 4) % RX_RING_LEN;
        for (index, byte) in out[..frame_len].iter_mut().enumerate() {
            *byte = ptr::read(ring.add((data_start + index) % RX_RING_LEN));
        }
        driver.rx_offset = (offset + 4 + length + 3) & !3;
        driver.rx_offset %= RX_RING_LEN;
        driver.write16(0x38, driver.rx_offset.wrapping_sub(16) as u16);
        Ok(Some(frame_len))
    }
}
