use crate::{
    TextGrid,
    color::{self, Color},
    graphics::{self, Framebuffer},
    idt::{PS2_COMMAND, PS2_DATA, PS2_STATUS, inb, outb},
    kprintln,
};
use spin::Mutex;

const MOUSE_CURSOR_SIZE: (usize, usize) = (5, 5);

/// Last drawn mouse cursor position, so erase_mouse_cursor knows what to
/// paint over. None until the first draw.
static LAST_MOUSE_POS: Mutex<Option<(usize, usize)>> = Mutex::new(None);
static SAVED_UNDER: Mutex<[Color; 20 * 20]> = Mutex::new([color::WHITE; 20 * 20]);

pub fn draw_mouse_cursor(fb: &Framebuffer, x: usize, y: usize) {
    if x <= fb.width || y <= fb.height {
        let mut saved = SAVED_UNDER.lock();

        for row in 0..20 {
            for col in 0..20 {
                saved[row * 20 + col] = unsafe { fb.read_pixel(x + col, y + row) };
            }
        }

        *LAST_MOUSE_POS.lock() = Some((x, y));
        graphics::draw_rec(fb, (x, y), MOUSE_CURSOR_SIZE, color::WHITE);
    }
}

pub fn erase_mouse_cursor(fb: &Framebuffer, grid: &TextGrid, char_w: usize, char_h: usize) {
    if let Some((x, y)) = LAST_MOUSE_POS.lock().take() {
        let start_col = x / char_w;
        let start_row = y / char_h;
        let end_col = (x + MOUSE_CURSOR_SIZE.0).div_ceil(char_w);
        let end_row = (y + MOUSE_CURSOR_SIZE.1).div_ceil(char_h);

        for row in start_row..end_row.min(grid.rows) {
            for col in start_col..end_col.min(grid.cols) {
                if let Some(cell) = grid.get(col, row) {
                    let cell_x = col * char_w;
                    let cell_y = row * char_h;
                    // black out just this cell first, then redraw the real char
                    graphics::draw_rec(fb, (cell_x, cell_y), (char_w, char_h), color::BLACK);
                    let mut buf = [0u8; 4];
                    graphics::draw_text(
                        fb,
                        cell.ch.encode_utf8(&mut buf),
                        (cell_x, cell_y),
                        cell.color,
                        None,
                    );
                }
            }
        }
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
    pub dx: i16,
    pub dy: i16,
    pub left: bool,
    pub right: bool,
    pub middle: bool,
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
    while unsafe { inb(PS2_STATUS) } & 0b10 != 0 {}
}
unsafe fn wait_read_ready() {
    // bit 0 of status = 1 means output buffer full (there's a byte for us to read)
    while unsafe { inb(PS2_STATUS) } & 0b01 == 0 {}
}

pub(crate) unsafe fn ps2_write_command(cmd: u8) {
    unsafe {
        wait_write_ready();
        outb(cmd, PS2_COMMAND);
    }
}

pub(crate) unsafe fn ps2_write_data(data: u8) {
    unsafe {
        wait_write_ready();
        outb(data, PS2_DATA);
    }
}

pub(crate) unsafe fn ps2_read_data() -> u8 {
    unsafe {
        wait_read_ready();
        return inb(PS2_DATA);
    }
}
