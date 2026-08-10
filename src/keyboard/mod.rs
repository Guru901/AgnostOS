use crate::platform::ring_buffer::RingBuffer;
use pc_keyboard::{
    DecodedKey, HandleControl, KeyCode, KeyState, PS2Keyboard, ScancodeSet1, layouts,
};
use spin::Mutex;

#[derive(Clone, Copy)]
pub(crate) struct Scancode(u8);

impl Scancode {
    #[must_use]
    pub(crate) const fn new(value: u8) -> Self {
        Self(value)
    }

    #[must_use]
    const fn get(self) -> u8 {
        self.0
    }
}

pub(crate) static KEYBOARD_QUEUE: Mutex<RingBuffer<Scancode, 256>> = Mutex::new(RingBuffer::new());

static KEYBOARD: Mutex<PS2Keyboard<layouts::Us104Key, ScancodeSet1>> =
    Mutex::new(PS2Keyboard::new(
        ScancodeSet1::new(),
        layouts::Us104Key,
        HandleControl::Ignore,
    ));

// Only `poll` touches this; no interrupt handler does, so no interrupt-safe
// access is needed. Revisit if a handler ever reads or writes it.
static CTRL_HELD: Mutex<bool> = Mutex::new(false);

pub(crate) enum KeyboardEvent {
    Char(char),
    CtrlC,
    ZoomIn,
    ZoomOut,
    ArrowUp,
    ArrowDown,
    CtrlL,
}

pub(crate) fn poll() -> Option<KeyboardEvent> {
    // An IRQ can preempt this code. Disable interrupts while holding the queue
    // lock so the handler never spins waiting for the interrupted code to unlock it.
    let scancode =
        x86_64::instructions::interrupts::without_interrupts(|| KEYBOARD_QUEUE.lock().pop())?;

    let mut kb = KEYBOARD.lock();

    let key_event = kb.add_byte(scancode.get()).ok()??;
    match key_event.code {
        KeyCode::LControl | KeyCode::RControl => {
            *CTRL_HELD.lock() = key_event.state == KeyState::Down;
            return None;
        }
        KeyCode::ArrowUp if key_event.state == KeyState::Down => {
            return Some(KeyboardEvent::ArrowUp);
        }
        KeyCode::ArrowDown if key_event.state == KeyState::Down => {
            return Some(KeyboardEvent::ArrowDown);
        }
        _ => {}
    }

    let ctrl = *CTRL_HELD.lock();

    if ctrl && key_event.state == KeyState::Down {
        match key_event.code {
            KeyCode::C => return Some(KeyboardEvent::CtrlC),
            KeyCode::L => return Some(KeyboardEvent::CtrlL),
            KeyCode::OemPlus => return Some(KeyboardEvent::ZoomIn),
            KeyCode::OemMinus => return Some(KeyboardEvent::ZoomOut),
            _ => {}
        }
    }

    if let Some(DecodedKey::Unicode(c)) = kb.process_keyevent(key_event) {
        return Some(KeyboardEvent::Char(c));
    }

    None
}
