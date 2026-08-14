use crate::platform::ring_buffer::RingBuffer;
use crate::{
    color::{self, Color},
    graphics::{self, Framebuffer, PixelCoord, PixelSize},
    idt::{PS2_COMMAND, PS2_DATA, PS2_STATUS, inb, outb},
};
use spin::Mutex;

pub(crate) const MOUSE_CURSOR_SIZE: PixelSize = PixelSize::new(5, 5);
const MOUSE_CURSOR_PIXELS: usize = 25;

/// The framebuffer pixels beneath the current mouse cursor.  Keeping the
/// position and pixels together prevents restoring a buffer from a different
/// cursor position.
struct MouseCursor {
    position: Option<(usize, usize)>,
    saved_under: [Color; MOUSE_CURSOR_PIXELS],
}

static MOUSE_CURSOR: Mutex<MouseCursor> = Mutex::new(MouseCursor {
    position: None,
    saved_under: [color::BLACK; MOUSE_CURSOR_PIXELS],
});

pub(crate) fn draw_mouse_cursor(fb: &Framebuffer, origin: PixelCoord) {
    let (x, y) = (origin.x(), origin.y());
    let Some(right) = x.checked_add(MOUSE_CURSOR_SIZE.width()) else {
        return;
    };
    let Some(bottom) = y.checked_add(MOUSE_CURSOR_SIZE.height()) else {
        return;
    };
    if !fb.is_drawable() || right > fb.width() || bottom > fb.height() {
        return;
    }

    let mut cursor = MOUSE_CURSOR.lock();

    for row in 0..MOUSE_CURSOR_SIZE.height() {
        for col in 0..MOUSE_CURSOR_SIZE.width() {
            // SAFETY: the cursor rectangle was checked to fit in this drawable
            // framebuffer, and `read_pixel` rechecks its byte offset.
            cursor.saved_under[row * MOUSE_CURSOR_SIZE.width() + col] =
                fb.read_pixel(x + col, y + row);
        }
    }

    cursor.position = Some((x, y));
    graphics::draw_rec(fb, PixelCoord::new(x, y), MOUSE_CURSOR_SIZE, color::WHITE);
}

pub(crate) fn erase_mouse_cursor(fb: &Framebuffer) {
    let mut cursor = MOUSE_CURSOR.lock();
    if let Some((x, y)) = cursor.position.take() {
        for row in 0..MOUSE_CURSOR_SIZE.height() {
            for col in 0..MOUSE_CURSOR_SIZE.width() {
                fb.write_pixel(
                    (y + row) * fb.stride() + (x + col),
                    &cursor.saved_under[row * MOUSE_CURSOR_SIZE.width() + col],
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;
    use uefi::proto::console::gop::PixelFormat;

    #[test]
    fn erasing_cursor_restores_the_pixels_it_covered() {
        let mut bytes = vec![0_u8; 7 * 7 * 4];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = index as u8;
        }
        let original = bytes.clone();
        // SAFETY: `bytes` remains alive and is not otherwise accessed while drawing.
        let fb = unsafe {
            Framebuffer::from_mut_slice(
                &mut bytes,
                PixelSize::new(7, 7),
                graphics::Stride::new(7),
                PixelFormat::Rgb,
            )
        }
        .unwrap();

        erase_mouse_cursor(&fb);
        draw_mouse_cursor(&fb, PixelCoord::new(1, 1));
        erase_mouse_cursor(&fb);

        assert_eq!(bytes, original);
    }
}

pub(crate) static MOUSE_QUEUE: Mutex<RingBuffer<u8, 256>> = Mutex::new(RingBuffer::new());

struct PacketState {
    bytes: [u8; 3],
    index: usize,
}

static PACKET_STATE: Mutex<PacketState> = Mutex::new(PacketState {
    bytes: [0; 3],
    index: 0,
});

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) struct MouseDelta(i16);

impl MouseDelta {
    #[must_use]
    pub(crate) const fn new(value: i16) -> Self {
        Self(value)
    }

    #[must_use]
    pub(crate) const fn get(self) -> i16 {
        self.0
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct MouseEvent {
    pub(crate) dx: MouseDelta,
    pub(crate) dy: MouseDelta,
    left: bool,
    right: bool,
    middle: bool,
}

pub(crate) fn poll() -> Option<MouseEvent> {
    let byte = crate::platform::without_interrupts(|| MOUSE_QUEUE.lock().pop())?;
    let mut state = PACKET_STATE.lock();

    // Byte 0 of a valid packet always has bit 3 set. If we're expecting a
    // byte 0 and don't see that bit, we're out of sync (e.g. missed a byte
    // to a full queue) — drop it and wait for a real packet start.
    if state.index == 0 && byte & 0b0000_1000 == 0 {
        return None;
    }

    let index = state.index;
    state.bytes[index] = byte;

    state.index += 1;

    if state.index < 3 {
        return None;
    }

    state.index = 0;
    let flags = state.bytes[0];
    if flags & 0b1100_0000 != 0 {
        return None;
    }
    let mut dx = state.bytes[1] as i16;
    let mut dy = state.bytes[2] as i16;

    // Sign-extend using the sign bits carried in byte 0.
    if flags & 0b0001_0000 != 0 {
        dx -= 256;
    }
    if flags & 0b0010_0000 != 0 {
        dy -= 256;
    }
    // PS/2 reports +Y as "up"; invert to match typical screen coordinates
    // where +Y is "down".
    dy = -dy;

    Some(MouseEvent {
        dx: MouseDelta::new(dx),
        dy: MouseDelta::new(dy),
        left: flags & 0b0000_0001 != 0,
        right: flags & 0b0000_0010 != 0,
        middle: flags & 0b0000_0100 != 0,
    })
}

const PS2_READY_POLL_LIMIT: usize = 100_000;
const CMD_ENABLE_AUX: u8 = 0xa8;
const CMD_READ_CONFIG: u8 = 0x20;
const CMD_WRITE_CONFIG: u8 = 0x60;
const CMD_WRITE_TO_AUX: u8 = 0xd4;
const MOUSE_ENABLE_PACKETS: u8 = 0xf4;
const MOUSE_ACK: u8 = 0xfa;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum Ps2Error {
    Timeout,
    UnexpectedResponse(u8),
}

unsafe fn wait_write_ready() -> Result<(), Ps2Error> {
    // bit 1 of status = 1 means input buffer full (controller hasn't read our last byte yet)
    // SAFETY: this helper is called only while communicating with the PS/2
    // controller, whose status register is at `PS2_STATUS`.
    for _ in 0..PS2_READY_POLL_LIMIT {
        // SAFETY: controller setup serializes access, and `PS2_STATUS` is the
        // readable legacy controller status port on the supported platform.
        if unsafe { inb(PS2_STATUS) } & 0b10 == 0 {
            return Ok(());
        }
    }
    Err(Ps2Error::Timeout)
}
unsafe fn wait_read_ready() -> Result<(), Ps2Error> {
    // bit 0 of status = 1 means output buffer full (there's a byte for us to read)
    // SAFETY: this helper is called only while communicating with the PS/2
    // controller, whose status register is at `PS2_STATUS`.
    for _ in 0..PS2_READY_POLL_LIMIT {
        // SAFETY: controller setup serializes access, and `PS2_STATUS` is the
        // readable legacy controller status port on the supported platform.
        if unsafe { inb(PS2_STATUS) } & 0b01 != 0 {
            return Ok(());
        }
    }
    Err(Ps2Error::Timeout)
}

pub(crate) unsafe fn ps2_write_command(cmd: u8) -> Result<(), Ps2Error> {
    // SAFETY: the caller serializes PS/2 controller commands and uses this only
    // on hardware with the legacy controller present.
    unsafe {
        wait_write_ready()?;
        outb(cmd, PS2_COMMAND);
    }
    Ok(())
}

pub(crate) unsafe fn ps2_write_data(data: u8) -> Result<(), Ps2Error> {
    // SAFETY: the caller serializes PS/2 controller data writes and uses this
    // only on hardware with the legacy controller present.
    unsafe {
        wait_write_ready()?;
        outb(data, PS2_DATA);
    }
    Ok(())
}

pub(crate) unsafe fn ps2_read_data() -> Result<u8, Ps2Error> {
    // SAFETY: the caller serializes PS/2 controller access and consumes data
    // only from the legacy controller data port.
    unsafe {
        wait_read_ready()?;
        Ok(inb(PS2_DATA))
    }
}

/// Configures the PS/2 auxiliary port and enables three-byte mouse packets.
///
/// # Safety
///
/// The caller must disable interrupts and serialize all PS/2 controller access
/// for the full command sequence.
pub(crate) unsafe fn initialize_controller() -> Result<(), Ps2Error> {
    unsafe {
        ps2_write_command(CMD_ENABLE_AUX)?;
        ps2_write_command(CMD_READ_CONFIG)?;
        let mut config = ps2_read_data()?;
        // Bit 1 enables IRQ12; bit 5 enables the auxiliary device clock.
        config |= 0b0000_0010;
        config &= !0b0010_0000;
        ps2_write_command(CMD_WRITE_CONFIG)?;
        ps2_write_data(config)?;
        ps2_write_command(CMD_WRITE_TO_AUX)?;
        ps2_write_data(MOUSE_ENABLE_PACKETS)?;
        let ack = ps2_read_data()?;
        if ack != MOUSE_ACK {
            return Err(Ps2Error::UnexpectedResponse(ack));
        }
    }
    Ok(())
}
