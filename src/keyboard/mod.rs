use core::arch::asm;
use pc_keyboard::{
    DecodedKey, HandleControl, KeyCode, KeyState, PS2Keyboard, ScancodeSet1, layouts,
};
use spin::Mutex;

unsafe fn inb(port: u16) -> u8 {
    let val: u8;
    // SAFETY: callers use the legacy PS/2 controller ports (0x64 and 0x60),
    // which are available while the kernel is running on the target hardware.
    unsafe {
        asm!("in al, dx", out("al") val, in("dx") port);
    }
    val
}

static KEYBOARD: Mutex<PS2Keyboard<layouts::Us104Key, ScancodeSet1>> =
    Mutex::new(PS2Keyboard::new(
        ScancodeSet1::new(),
        layouts::Us104Key,
        HandleControl::Ignore,
    ));

static CTRL_HELD: Mutex<bool> = Mutex::new(false);

pub enum KeyboardEvent {
    Char(char),
    CtrlC,
    ZoomIn,
    ZoomOut,
    ArrowUp,
    ArrowDown,
    CtrlL,
}

pub fn poll() -> Option<KeyboardEvent> {
    // SAFETY: `inb` only accesses the PS/2 status and data ports; reading the
    // status port first ensures a data-port read is performed only when data is ready.
    unsafe {
        let status = inb(0x64);
        if status & 1 == 0 {
            return None; // no data waiting
        }

        let scancode = inb(0x60);
        let mut kb = KEYBOARD.lock();

        if let Ok(Some(key_event)) = kb.add_byte(scancode) {
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

            // normal character decoding
            if let Some(DecodedKey::Unicode(c)) = kb.process_keyevent(key_event) {
                return Some(KeyboardEvent::Char(c));
            }
        }
    }
    None
}
