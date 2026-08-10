use crate::{
    color::{self, Color},
    graphics::{self, Framebuffer},
    idt::{PS2_COMMAND, PS2_DATA, PS2_STATUS, inb, outb},
};
use spin::Mutex;

pub(crate) const MOUSE_CURSOR_SIZE: (usize, usize) = (5, 5);
const MOUSE_CURSOR_PIXELS: usize = MOUSE_CURSOR_SIZE.0 * MOUSE_CURSOR_SIZE.1;

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

pub fn draw_mouse_cursor(fb: &Framebuffer, x: usize, y: usize) {
    let Some(right) = x.checked_add(MOUSE_CURSOR_SIZE.0) else {
        return;
    };
    let Some(bottom) = y.checked_add(MOUSE_CURSOR_SIZE.1) else {
        return;
    };
    if !fb.is_drawable() || right > fb.width || bottom > fb.height {
        return;
    }

    let mut cursor = MOUSE_CURSOR.lock();

    for row in 0..MOUSE_CURSOR_SIZE.1 {
        for col in 0..MOUSE_CURSOR_SIZE.0 {
            // SAFETY: the cursor rectangle was checked to fit in this drawable
            // framebuffer, and `read_pixel` rechecks its byte offset.
            cursor.saved_under[row * MOUSE_CURSOR_SIZE.0 + col] =
                unsafe { fb.read_pixel(x + col, y + row) };
        }
    }

    cursor.position = Some((x, y));
    graphics::draw_rec(fb, (x, y), MOUSE_CURSOR_SIZE, color::WHITE);
}

pub fn erase_mouse_cursor(fb: &Framebuffer) {
    let mut cursor = MOUSE_CURSOR.lock();
    if let Some((x, y)) = cursor.position.take() {
        for row in 0..MOUSE_CURSOR_SIZE.1 {
            for col in 0..MOUSE_CURSOR_SIZE.0 {
                // SAFETY: `write_pixel` validates the computed pixel range before
                // performing its raw framebuffer write.
                unsafe {
                    fb.write_pixel(
                        (y + row) * fb.stride + (x + col),
                        &cursor.saved_under[row * MOUSE_CURSOR_SIZE.0 + col],
                    )
                };
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
        let fb = Framebuffer {
            ptr: bytes.as_mut_ptr(),
            width: 7,
            height: 7,
            stride: 7,
            pixel_format: PixelFormat::Rgb,
            byte_len: bytes.len(),
        };

        erase_mouse_cursor(&fb);
        draw_mouse_cursor(&fb, 1, 1);
        erase_mouse_cursor(&fb);

        assert_eq!(bytes, original);
    }
}

pub struct MouseQueue {
    buf: [u8; 256],
    head: usize,
    tail: usize,
}

pub static MOUSE_QUEUE: Mutex<MouseQueue> = Mutex::new(MouseQueue::new());

impl MouseQueue {
    const fn new() -> Self {
        Self {
            buf: [0; 256],
            head: 0,
            tail: 0,
        }
    }

    pub fn push(&mut self, byte: u8) {
        let next = (self.tail + 1) % self.buf.len();
        // Queue full
        if next == self.head {
            return;
        }
        self.buf[self.tail] = byte;
        self.tail = next;
    }

    pub fn pop(&mut self) -> Option<u8> {
        if self.head == self.tail {
            return None;
        }
        let byte = self.buf[self.head];
        self.head = (self.head + 1) % self.buf.len();
        Some(byte)
    }
}

struct PacketState {
    bytes: [u8; 3],
    index: usize,
}

static PACKET_STATE: Mutex<PacketState> = Mutex::new(PacketState {
    bytes: [0; 3],
    index: 0,
});

#[derive(Debug, Clone, Copy)]
pub struct MouseEvent {
    pub(crate) dx: i16,
    pub(crate) dy: i16,
    left: bool,
    right: bool,
    middle: bool,
}

pub fn poll() -> Option<MouseEvent> {
    let byte = x86_64::instructions::interrupts::without_interrupts(|| MOUSE_QUEUE.lock().pop())?;
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
        dx,
        dy,
        left: flags & 0b0000_0001 != 0,
        right: flags & 0b0000_0010 != 0,
        middle: flags & 0b0000_0100 != 0,
    })
}

unsafe fn wait_write_ready() {
    // bit 1 of status = 1 means input buffer full (controller hasn't read our last byte yet)
    // SAFETY: this helper is called only while communicating with the PS/2
    // controller, whose status register is at `PS2_STATUS`.
    while unsafe { inb(PS2_STATUS) } & 0b10 != 0 {}
}
unsafe fn wait_read_ready() {
    // bit 0 of status = 1 means output buffer full (there's a byte for us to read)
    // SAFETY: this helper is called only while communicating with the PS/2
    // controller, whose status register is at `PS2_STATUS`.
    while unsafe { inb(PS2_STATUS) } & 0b01 == 0 {}
}

pub(crate) unsafe fn ps2_write_command(cmd: u8) {
    // SAFETY: the caller serializes PS/2 controller commands and uses this only
    // on hardware with the legacy controller present.
    unsafe {
        wait_write_ready();
        outb(cmd, PS2_COMMAND);
    }
}

pub(crate) unsafe fn ps2_write_data(data: u8) {
    // SAFETY: the caller serializes PS/2 controller data writes and uses this
    // only on hardware with the legacy controller present.
    unsafe {
        wait_write_ready();
        outb(data, PS2_DATA);
    }
}

pub(crate) unsafe fn ps2_read_data() -> u8 {
    // SAFETY: the caller serializes PS/2 controller access and consumes data
    // only from the legacy controller data port.
    unsafe {
        wait_read_ready();
        inb(PS2_DATA)
    }
}
