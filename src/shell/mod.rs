//! Shell module — interactive command-line interface for AgnostOS.
//!
//! Provides a polling input loop that reads keyboard events and dispatches
//! them to the appropriate console or command handler. Commands are parsed
//! into a name, optional flags (prefixed with `-`), and positional arguments.

use alloc::string::String;

#[cfg(feature = "mouse")]
use crate::mouse;
use crate::{
    PROMPT,
    commands::run_command,
    console,
    keyboard::{self, KeyboardEvent},
    kprint, kprintln,
};

#[cfg(feature = "mouse")]
use crate::graphics::PixelCoord;

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
pub fn init() -> ! {
    console::clear_background();
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
            if let Some(event) = mouse::poll() {
                use crate::{CURSOR_H, CURSOR_W};
                console::with_framebuffer(|fb| {
                    mouse::erase_mouse_cursor(fb);
                    mouse_x = (mouse_x + event.dx.get() as i32)
                        .clamp(0, fb.width() as i32 - CURSOR_W as i32);
                    mouse_y = (mouse_y + event.dy.get() as i32)
                        .clamp(0, fb.height() as i32 - CURSOR_H as i32);
                    mouse::draw_mouse_cursor(
                        fb,
                        PixelCoord::new(mouse_x as usize, mouse_y as usize),
                    );
                });
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
                KeyboardEvent::ArrowRight => console::arrow_right(),
                KeyboardEvent::ArrowLeft => console::arrow_left(),
                KeyboardEvent::ArrowUp => console::arrow_up(&mut line),
                KeyboardEvent::ArrowDown => console::arrow_down(&mut line),
                KeyboardEvent::CtrlL => {
                    console::reset();
                    line.clear();
                    kprint!("{PROMPT}");
                }

                KeyboardEvent::Tab => console::auto_complete(&mut line),
            }

            console::draw_cursor();
        }
    }
}
