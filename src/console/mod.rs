//! Console module — stateful text output for the AgnostOS kernel.
//!
//! Manages a [`KWriter`] instance that owns the framebuffer reference,
//! cursor position, font size, and screen/command history. All text
//! output goes through [`_kprint`] (via the [`kprint!`]/[`kprintln!`] macros),
//! which acquires the [`KWRITER`] lock and calls [`fmt::Write`] on it.
//!
//! # Features
//! - Scrolling terminal output with configurable scroll threshold
//! - Command history navigation (up/down arrow keys)
//! - Font size switching with history redraw (zoom in/out)
//! - Block cursor rendering
//! - Backspace with visual erase

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;
use noto_sans_mono_bitmap::{RasterHeight, get_raster_width};
use spin::Mutex;

use crate::{
    FONT_WEIGHT, PROMPT, color,
    graphics::{self, Framebuffer},
};

/// Internal writer state. Holds everything needed to render text to the
/// framebuffer and track terminal state across keystrokes.
struct KWriter {
    /// Raw framebuffer to draw into.
    fb: Framebuffer,
    /// Current cursor position in.
    column: usize,
    /// Current cursor position in.
    row: usize,
    /// Screen cells occupied by the glyphs in `current_line`, in draw order.
    /// Keeping these positions lets backspace erase the cell that was actually
    /// rendered, including when the line crossed a wrap boundary.
    glyph_cells: Vec<Cell>,
    /// All lines that have been completed (ended with `\n`) since the last
    /// [`reset`]. Used for zoom redraw and command history recall.
    history: Vec<String>,
    /// The line currently being accumulated (not yet terminated by `\n`).
    current_line: String,
    /// Active font size. Affects glyph rendering and cursor/line spacing.
    font_size: RasterHeight,
    /// Index into the command history for up/down arrow recall.
    /// `None` means the user is not currently browsing history.
    history_index: Option<usize>,
}

/// A terminal-cell position, independent of its pixel representation.
#[derive(Clone, Copy)]
struct Cell {
    column: usize,
    row: usize,
}

// SAFETY: Single-core kernel — no actual concurrent access occurs.
// These impls satisfy Rust's type system requirements for a static global.
unsafe impl Send for KWriter {}

static KWRITER: Mutex<Option<KWriter>> = Mutex::new(None);

/// Returns the line height in pixels for the given font size, including
/// 2px of line spacing.
fn font_h(size: RasterHeight) -> usize {
    let px = match size {
        RasterHeight::Size16 => 16,
        RasterHeight::Size20 => 20,
        RasterHeight::Size24 => 24,
        RasterHeight::Size32 => 32,
    };
    px + 2
}

/// Returns the character advance width in pixels for the given font size.
fn font_w(size: RasterHeight) -> usize {
    get_raster_width(FONT_WEIGHT, size)
}

/// Initializes the console with the given framebuffer.
///
/// Must be called before any [`kprint!`] or [`kprintln!`] calls.
/// Clones the framebuffer so the console owns its own copy of the
/// raw pointer and dimensions.
pub fn init(fb: &Framebuffer) {
    let fb = fb.clone();
    *KWRITER.lock() = Some(KWriter {
        fb,
        column: 0,
        row: 0,
        glyph_cells: Vec::new(),
        font_size: RasterHeight::Size16,
        current_line: String::new(),
        history: Vec::new(),
        history_index: None,
    });
}

impl KWriter {
    fn x(&self) -> usize {
        return self.column * font_w(self.font_size);
    }

    fn y(&self) -> usize {
        return self.row * font_h(self.font_size);
    }

    /// Scrolls the framebuffer up if the cursor has reached the scroll
    /// threshold (3 lines from the bottom). Adjusts `self.y` accordingly.
    fn check_scroll(&mut self, fh: usize) {
        let threshold = self.fb.height.saturating_sub(3 * fh);
        if self.y() >= threshold {
            let scroll_by = self.y() - threshold + fh;
            graphics::scroll_up(&self.fb, scroll_by);
            self.row = threshold.saturating_sub(fh) / fh;
        }
    }
}

impl fmt::Write for KWriter {
    /// Writes a string to the framebuffer, handling newlines, line wrapping,
    /// and scrolling. Each completed line (terminated by `\n`) is pushed
    /// into `history`.
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let width = self.fb.width;
        let fh = font_h(self.font_size);
        let fw = font_w(self.font_size);

        for ch in s.chars() {
            if ch == '\n' {
                // flush current line into history and move cursor down
                self.history.push(core::mem::take(&mut self.current_line));
                self.glyph_cells.clear();
                self.column = 0;
                self.row += 1;
                self.check_scroll(fh);
                continue;
            }

            self.current_line.push(ch);

            // wrap to next line if we've reached the right edge
            if (self.column + 1) * fw > width {
                self.column = 0;
                self.row += 1;
            }

            let mut buf = [0u8; 4];
            let s = ch.encode_utf8(&mut buf);
            graphics::draw_text(
                &self.fb,
                s,
                (self.x(), self.y()),
                color::WHITE,
                Some(self.font_size),
            );
            self.glyph_cells.push(Cell {
                column: self.column,
                row: self.row,
            });
            self.column += 1;
        }

        Ok(())
    }
}

/// Internal print function — acquires the [`KWRITER`] lock and calls
/// [`fmt::Write::write_fmt`]. Use the [`kprint!`] and [`kprintln!`] macros
/// instead of calling this directly.
pub fn kprint(args: fmt::Arguments) {
    use fmt::Write;
    if let Some(writer) = KWRITER.lock().as_mut() {
        writer.write_fmt(args).ok();
    }
}

/// Prints to the kernel console without a trailing newline.
#[macro_export]
macro_rules! kprint {
    ($($arg:tt)*) => {
        $crate::console::kprint(format_args!($($arg)*))
    };
}

/// Prints to the kernel console with a trailing newline.
#[macro_export]
macro_rules! kprintln {
    () => ($crate::kprint!("\n"));
    ($($arg:tt)*) => {{
        $crate::kprint!($($arg)*);
        $crate::kprint!("\n");
    }};
}

/// Clears the screen, resets the cursor to the top-left, and wipes all
/// history. Called by the `clear` shell command and Ctrl+L.
pub(crate) fn reset() {
    if let Some(writer) = KWRITER.lock().as_mut() {
        graphics::clear_background(&writer.fb, &color::BLACK);
        writer.column = 0;
        writer.row = 0;
        writer.glyph_cells.clear();
        writer.history.clear();
        writer.current_line.clear();
        writer.history_index = None;
    }
}

/// Erases the last typed character from the screen and moves the cursor back.
/// Handles wrapping back to the previous line if the cursor is at x=0.
pub(crate) fn backspace() {
    if let Some(writer) = KWRITER.lock().as_mut() {
        let fh = font_h(writer.font_size);
        let fw = font_w(writer.font_size);

        // keep logical input in sync with erased glyph
        if writer.current_line.chars().count() <= PROMPT.chars().count() {
            return;
        }
        writer.current_line.pop();

        let Some(cell) = writer.glyph_cells.pop() else {
            return;
        };
        writer.column = cell.column;
        writer.row = cell.row;

        // paint over the erased character with background color
        crate::graphics::draw_rec(
            &writer.fb,
            (writer.x(), writer.y()),
            (fw, fh),
            crate::color::BLACK,
        );
    }
}

/// Draws a block cursor at the current cursor position.
pub(crate) fn draw_cursor() {
    if let Some(writer) = KWRITER.lock().as_mut() {
        let fh = font_h(writer.font_size);
        let fw = font_w(writer.font_size);

        if writer.fb.width < writer.x() + fw {
            writer.column = 0;
            writer.row += 1;
            writer.check_scroll(fh);
        }

        crate::graphics::draw_rec(
            &writer.fb,
            (writer.x(), writer.y()),
            (fw, fh - 4),
            crate::color::WHITE,
        );
    }
}

/// Erases the block cursor at the current cursor position by painting
/// over it with the background color.
pub(crate) fn erase_cursor() {
    if let Some(writer) = KWRITER.lock().as_mut() {
        let fh = font_h(writer.font_size);
        let fw = font_w(writer.font_size);
        crate::graphics::draw_rec(
            &writer.fb,
            (writer.x(), writer.y()),
            (fw, fh - 4),
            crate::color::BLACK,
        );
    }
}

/// Redraws the visible portion of history onto the framebuffer.
///
/// Only the most recent lines that fit on screen are shown. Used after
/// zoom changes to re-render history at the new font size.
pub(crate) fn print_history() {
    if let Some(writer) = KWRITER.lock().as_mut() {
        let fh = font_h(writer.font_size);
        let max_lines = writer.fb.height / fh;

        // only render the tail of history that fits on screen
        let start = writer.history.len().saturating_sub(max_lines);
        let visible = &writer.history[start..];

        let mut y = 0usize;
        for line in visible {
            graphics::draw_text(
                &writer.fb,
                line,
                (0, y),
                color::WHITE,
                Some(writer.font_size),
            );
            y += fh;
        }

        writer.column = 0;
        writer.row = y / fh;
    } else {
        kprintln!("Failed to print history");
    }
}

/// Increases the font size by one step (up to Size32) and redraws history.
pub(crate) fn zoom_in(current_input: &str) {
    if let Some(writer) = KWRITER.lock().as_mut() {
        writer.font_size = match writer.font_size {
            RasterHeight::Size16 => RasterHeight::Size20,
            RasterHeight::Size20 => RasterHeight::Size24,
            RasterHeight::Size24 | RasterHeight::Size32 => RasterHeight::Size32, // already at max
        };
        graphics::clear_background(&writer.fb, &color::BLACK);
    }
    print_history();
    kprint!("{PROMPT}{current_input}");
}

/// Decreases the font size by one step (down to Size16) and redraws history.
pub(crate) fn zoom_out(current_input: &str) {
    if let Some(writer) = KWRITER.lock().as_mut() {
        writer.font_size = match writer.font_size {
            RasterHeight::Size32 => RasterHeight::Size24,
            RasterHeight::Size24 => RasterHeight::Size20,
            RasterHeight::Size20 | RasterHeight::Size16 => RasterHeight::Size16, // already at min
        };
        graphics::clear_background(&writer.fb, &color::BLACK);
    }
    print_history();
    kprint!("{PROMPT}{current_input}");
}

/// Returns only the command lines from history (lines starting with [`PROMPT`]),
/// excluding empty lines and `^C` entries. Used for up/down arrow history recall.
fn command_history(history: &[String]) -> Vec<&String> {
    history
        .iter()
        .filter(|line| line.starts_with(PROMPT) && line.as_str() != "^C" && !line.is_empty())
        .collect()
}

/// Navigates one step back in command history, updating the input line
/// on screen and in the provided `current_line` buffer.
pub(crate) fn arrow_up(current_line: &mut String) {
    if let Some(writer) = KWRITER.lock().as_mut() {
        let commands = command_history(&writer.history);

        if commands.is_empty() {
            return;
        }

        let new_index = match writer.history_index {
            None => commands.len() - 1, // start from most recent
            Some(0) => 0,               // already at oldest, clamp
            Some(i) => i - 1,
        };

        writer.history_index = Some(new_index);

        let recalled = commands[new_index].trim_start_matches(PROMPT).to_string();
        redraw_input_line(writer, &recalled);
        current_line.clear();
        current_line.push_str(&recalled);
    }
}

/// Navigates one step forward in command history. If past the newest entry,
/// clears the input line (restoring empty prompt).
pub(crate) fn arrow_down(current_line: &mut String) {
    if let Some(writer) = KWRITER.lock().as_mut() {
        let commands = command_history(&writer.history);

        if commands.is_empty() {
            return;
        }

        let new_text = match writer.history_index {
            None => return, // not currently browsing, nothing to do
            Some(i) if i + 1 < commands.len() => {
                writer.history_index = Some(i + 1);
                commands[i + 1].trim_start_matches(PROMPT).to_string()
            }
            Some(_) => {
                writer.history_index = None; // past newest — restore empty input
                String::new()
            }
        };

        redraw_input_line(writer, &new_text);
        current_line.clear();
        current_line.push_str(&new_text);
    }
}

/// Redraws the current input line in place — erases the row, redraws the
/// prompt, then draws `text` after it. Updates `writer.x` to the end of
/// the new text so the cursor lands in the right place.
fn redraw_input_line(writer: &mut KWriter, text: &str) {
    let fh = font_h(writer.font_size);
    let fw = font_w(writer.font_size);

    // erase the entire current row
    graphics::draw_rec(
        &writer.fb,
        (0, writer.y()),
        (writer.fb.width, fh),
        color::BLACK,
    );

    graphics::draw_text(
        &writer.fb,
        PROMPT,
        (0, writer.y()),
        color::WHITE,
        Some(writer.font_size),
    );

    graphics::draw_text(
        &writer.fb,
        text,
        (fw * PROMPT.chars().count(), writer.y()),
        color::WHITE,
        Some(writer.font_size),
    );

    writer.current_line.clear();
    writer.current_line.push_str(PROMPT);
    writer.current_line.push_str(text);
    writer.glyph_cells.clear();
    writer
        .glyph_cells
        .extend((0..writer.current_line.chars().count()).map(|column| Cell {
            column,
            row: writer.row,
        }));
    writer.column = writer.current_line.chars().count();
}

/// Sets the font size directly without redrawing. Use [`zoom_in`]/[`zoom_out`]
/// for size changes that should also redraw history. This is used by the
/// `font` shell command.
pub(crate) fn set_font_size(font_size: RasterHeight) {
    if let Some(writer) = KWRITER.lock().as_mut() {
        writer.font_size = font_size;
    }
}
