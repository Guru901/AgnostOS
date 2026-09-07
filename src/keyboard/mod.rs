use core::sync::atomic::{AtomicUsize, Ordering};

use pc_keyboard::{
    DecodedKey, HandleControl, KeyCode, KeyState, PS2Keyboard, ScancodeSet1, layouts,
};
use ringbuf::{
    StaticCons, StaticProd, StaticRb,
    traits::{Consumer, Producer, SplitRef},
};
use spin::Mutex;
use static_cell::StaticCell;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct KeyboardScancode(u8);

impl KeyboardScancode {
    #[must_use]
    pub(crate) const fn new(value: u8) -> Self {
        Self(value)
    }

    #[must_use]
    #[allow(dead_code)]
    const fn get(self) -> u8 {
        self.0
    }
}

const RB_SIZE: usize = 256;

pub(crate) static KEYBOARD_DROPPED: AtomicUsize = AtomicUsize::new(0);

// Lock-free single-producer/single-consumer queue: the IRQ handler owns the
// producer and the shell loop owns the consumer. Keep those ownership rules
// intact unless the queue is redesigned for multiple producers/consumers.
pub(crate) static KEYBOARD_QUEUE: StaticCell<StaticRb<KeyboardScancode, RB_SIZE>> =
    StaticCell::new();
pub(crate) static KEYBOARD_PRODUCER_CELL: StaticCell<
    StaticProd<'static, KeyboardScancode, RB_SIZE>,
> = StaticCell::new();
pub(crate) static KEYBOARD_CONSUMER_CELL: StaticCell<
    StaticCons<'static, KeyboardScancode, RB_SIZE>,
> = StaticCell::new();

// filled in once at startup, read from thereafter without re-locking
static mut KEYBOARD_PRODUCER: Option<&'static mut StaticProd<'static, KeyboardScancode, RB_SIZE>> =
    None;
static mut KEYBOARD_CONSUMER: Option<&'static mut StaticCons<'static, KeyboardScancode, RB_SIZE>> =
    None;

pub(crate) fn init_keyboard() {
    let rb = KEYBOARD_QUEUE.init(StaticRb::default());
    let (producer, consumer) = rb.split_ref();

    // SAFETY: this function runs exactly once, before interrupts are enabled
    // and before the producer/consumer can be accessed from anywhere else,
    // so there is no concurrent access to PRODUCER/CONSUMER at this point.
    unsafe {
        KEYBOARD_PRODUCER = Some(KEYBOARD_PRODUCER_CELL.init(producer));
        KEYBOARD_CONSUMER = Some(KEYBOARD_CONSUMER_CELL.init(consumer));
    }
}

// call from wherever pushes (e.g. keyboard interrupt handler)
pub(crate) fn push_keyboard_scancode(code: KeyboardScancode) {
    // SAFETY: PRODUCER is only ever accessed from this function, which is only
    // ever called from the keyboard interrupt handler. That handler cannot
    // preempt itself (interrupts of the same priority don't nest), so this is
    // the sole, non-reentrant writer of PRODUCER — no other code reads or
    // writes it, so there is no data race despite the raw static access.
    unsafe {
        if let Some(p) = &mut *core::ptr::addr_of_mut!(KEYBOARD_PRODUCER)
            && p.try_push(code).is_err()
        {
            KEYBOARD_DROPPED.fetch_add(1, Ordering::Relaxed);
        }
    }
}

// call from wherever pops (e.g. main loop)
pub(crate) fn pop_keyboard_scancode() -> Option<KeyboardScancode> {
    // SAFETY: CONSUMER is only ever accessed from this function, which is only
    // ever called from the main loop (never from an interrupt context), so
    // there is a single, non-reentrant reader/writer of CONSUMER and no
    // concurrent access with the disjoint producer.
    unsafe {
        (&mut *core::ptr::addr_of_mut!(KEYBOARD_CONSUMER))
            .as_mut()?
            .try_pop()
    }
}

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
    ArrowLeft,
    ArrowRight,
    CtrlL,
    Tab,
}

pub(crate) fn poll() -> Option<KeyboardEvent> {
    // An IRQ can preempt this code. Disable interrupts while holding the queue
    // lock so the handler never spins waiting for the interrupted code to unlock it.
    //let scancode = without_interrupts(|| KEYBOARD_QUEUE.lock().pop())?;
    let scancode = pop_keyboard_scancode()?;
    let mut kb = KEYBOARD.lock();

    let key_event = kb.add_byte(scancode.0).ok()??;
    match key_event.code {
        KeyCode::LControl | KeyCode::RControl => {
            *CTRL_HELD.lock() = key_event.state == KeyState::Down;
            return None;
        }

        KeyCode::ArrowLeft if key_event.state == KeyState::Down => {
            return Some(KeyboardEvent::ArrowLeft);
        }

        KeyCode::ArrowRight if key_event.state == KeyState::Down => {
            return Some(KeyboardEvent::ArrowRight);
        }

        KeyCode::ArrowUp if key_event.state == KeyState::Down => {
            return Some(KeyboardEvent::ArrowUp);
        }

        KeyCode::ArrowDown if key_event.state == KeyState::Down => {
            return Some(KeyboardEvent::ArrowDown);
        }

        KeyCode::Tab if key_event.state == KeyState::Down => {
            return Some(KeyboardEvent::Tab);
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

#[cfg(test)]
mod queue_tests {
    use super::*;
    use ringbuf::traits::{Consumer, Producer, SplitRef};

    #[test]
    fn decoder_handles_printable_keys_modifiers_and_arrows() {
        let mut decoder = PS2Keyboard::new(
            ScancodeSet1::new(),
            layouts::Us104Key,
            HandleControl::Ignore,
        );

        let key = decoder.add_byte(0x1e).unwrap().unwrap();
        assert_eq!(key.code, KeyCode::A);
        assert_eq!(key.state, KeyState::Down);

        let control = decoder.add_byte(0x1d).unwrap().unwrap();
        assert_eq!(control.code, KeyCode::LControl);
        assert_eq!(control.state, KeyState::Down);

        let arrow_up = decoder.add_byte(0xe0).unwrap();
        assert!(arrow_up.is_none());
        let arrow_up = decoder.add_byte(0x48).unwrap().unwrap();
        assert_eq!(arrow_up.code, KeyCode::ArrowUp);
        assert_eq!(arrow_up.state, KeyState::Down);
    }

    #[test]
    fn queue_preserves_fifo_order_across_wraparound() {
        let mut queue = StaticRb::<KeyboardScancode, 2>::default();
        let (mut producer, mut consumer) = queue.split_ref();

        producer.try_push(KeyboardScancode::new(1)).unwrap();
        producer.try_push(KeyboardScancode::new(2)).unwrap();
        assert_eq!(consumer.try_pop().map(|code| code.0), Some(1));
        producer.try_push(KeyboardScancode::new(3)).unwrap();

        assert_eq!(consumer.try_pop().map(|code| code.0), Some(2));
        assert_eq!(consumer.try_pop().map(|code| code.0), Some(3));
        assert_eq!(consumer.try_pop(), None);
    }

    #[test]
    fn full_queue_rejects_new_scancodes_and_counts_them() {
        let mut queue = StaticRb::<KeyboardScancode, 2>::default();
        let (mut producer, _) = queue.split_ref();
        let mut dropped = 0;

        producer.try_push(KeyboardScancode::new(1)).unwrap();
        producer.try_push(KeyboardScancode::new(2)).unwrap();
        if producer.try_push(KeyboardScancode::new(3)).is_err() {
            dropped += 1;
        }

        assert_eq!(dropped, 1);
    }
}
