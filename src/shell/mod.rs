//! Shell module — interactive command-line interface for AgnostOS.
//!
//! Provides a polling input loop that reads keyboard events and dispatches
//! them to the appropriate console or command handler. Commands are parsed
//! into a name, optional flags (prefixed with `-`), and positional arguments.

use alloc::string::String;

#[cfg(feature = "mouse")]
use crate::mouse;
use crate::{
    PROMPT, color,
    commands::run_command,
    console,
    graphics::{self, Framebuffer},
    keyboard::{self, KeyboardEvent},
    kprint, kprintln,
};

/// Initializes and runs the interactive shell. Never returns (`-> !`).
///
/// Clears the screen, prints the initial prompt, and enters a polling loop
/// that reads keyboard events and dispatches them:
///
/// - Printable characters are echoed and appended to the current line buffer.
/// - Enter runs the current line as a command via [`run_command`].
/// - Backspace erases the last character both from the buffer and the screen.
/// - Ctrl+C cancels the current line.
/// - Ctrl+L clears the screen.
/// - Arrow up/down navigate command history.
/// - Ctrl+Plus/Minus zoom the font in/out.
pub fn init(fb: &Framebuffer) -> ! {
    graphics::clear_background(fb, &color::BLACK);
    let mut line = String::new();
    #[cfg(feature = "mouse")]
    let mut mouse_x: i32 = 0;
    #[cfg(feature = "mouse")]
    let mut mouse_y: i32 = 0;

    kprint!("{PROMPT}");
    console::draw_cursor();

    loop {
        #[cfg(feature = "mouse")]
        {
            let mut mouse_moved = false;
            mouse::drain(|event| {
                mouse_moved = true;
                // Keep applying the complete batch before redrawing.  This avoids
                // rendering stale queued deltas one at a time after direction has
                // already changed.
                mouse_x = mouse_x.saturating_add(event.dx as i32).clamp(
                    0,
                    fb.width.saturating_sub(mouse::MOUSE_CURSOR_SIZE.0) as i32,
                );
                mouse_y = mouse_y.saturating_add(event.dy as i32).clamp(
                    0,
                    fb.height.saturating_sub(mouse::MOUSE_CURSOR_SIZE.1) as i32,
                );
            });
            if mouse_moved {
                mouse::erase_mouse_cursor(fb);
                // Clamp using the actual pointer dimensions. CURSOR_W/H describe
                // the text cursor and left an invisible 15-pixel dead zone at the
                // right and bottom edges for this 5x5 mouse cursor.
                mouse::draw_mouse_cursor(fb, mouse_x as usize, mouse_y as usize);
            }
        }
        if let Some(key) = keyboard::poll() {
            console::erase_cursor();

            match key {
                KeyboardEvent::Char(c) => match c {
                    '\n' => {
                        kprintln!();
                        run_command(&line);
                        line.clear();
                        kprint!("{PROMPT}");
                    }
                    '\u{8}' => {
                        // backspace — remove last char from buffer and erase from screen
                        if line.pop().is_some() {
                            console::backspace();
                        }
                    }
                    c => {
                        line.push(c);
                        kprint!("{}", c);
                    }
                },
                KeyboardEvent::CtrlC => {
                    kprintln!("^C");
                    line.clear();
                    kprint!("{PROMPT}");
                }
                KeyboardEvent::ZoomIn => console::zoom_in(&line),
                KeyboardEvent::ZoomOut => console::zoom_out(&line),
                KeyboardEvent::ArrowUp => console::arrow_up(&mut line),
                KeyboardEvent::ArrowDown => console::arrow_down(&mut line),
                KeyboardEvent::CtrlL => {
                    console::reset();
                    line.clear();
                    kprint!("{PROMPT}");
                }
            }

            console::draw_cursor();
        }
    }
}
