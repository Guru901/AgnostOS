//! Shell module — interactive command-line interface for AgnostOS.
//!
//! Provides a polling input loop that reads keyboard events and dispatches
//! them to the appropriate console or command handler. Commands are parsed
//! into a name, optional flags (prefixed with `-`), and positional arguments.

use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::Ordering;
use noto_sans_mono_bitmap::RasterHeight;

use crate::{
    HEAP_SIZE, HEAP_START, PROMPT, color, console,
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

    kprint!("{PROMPT}");
    console::draw_cursor();

    loop {
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
                KeyboardEvent::ZoomIn => console::zoom_in(),
                KeyboardEvent::ZoomOut => console::zoom_out(),
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

enum Commands {
    HELP,
    ABOUT,
    HISTORY,
    ECHO,
    MEMINFO,
    FONT,
    CLEAR,
    SHUTDOWN,
    EMPTY,
    UNKNOWN,
}

impl From<&str> for Commands {
    fn from(value: &str) -> Self {
        match value {
            "help" => Commands::HELP,
            "about" => Commands::ABOUT,
            "history" => Commands::HISTORY,
            "echo" => Commands::ECHO,
            "meminfo" => Commands::MEMINFO,
            "font" => Commands::FONT,
            "clear" => Commands::CLEAR,
            "shutdown" => Commands::SHUTDOWN,
            "" => Commands::EMPTY,
            _ => Commands::UNKNOWN,
        }
    }
}

/// Parses and dispatches a command string.
///
/// Splits the input into a command name, flags (tokens starting with `-`),
/// and positional arguments. Dispatches to the appropriate handler or prints
/// "Unknown command" if the command is not recognized.
fn run_command(command: &str) {
    let command = command.trim();
    let mut iter = command.split_whitespace();
    let command = iter.next().unwrap_or("");
    let command = Commands::from(command);

    // separate flags (e.g. --verbose) from positional args
    let mut flags: Vec<&str> = Vec::new();
    let mut args: Vec<&str> = Vec::new();
    for part in iter {
        if part.starts_with('-') {
            flags.push(part);
        } else {
            args.push(part);
        }
    }
    let _ = flags; // flags parsed but reserved for future use

    match command {
        Commands::HELP => help(&args),
        Commands::ABOUT => {
            kprintln!("AgnostOS v0.1 - written in Rust \n codeberg.com/guru901/agnostos")
        }
        Commands::HISTORY => console::print_history(),
        Commands::ECHO => kprintln!("{}", args.join(" ")),
        Commands::MEMINFO => {
            let start = HEAP_START.load(Ordering::Relaxed);
            let size = HEAP_SIZE.load(Ordering::Relaxed);
            kprintln!("heap start: {:#x}", start);
            kprintln!("heap size:  {}mb", size / (1024 * 1024));
        }
        Commands::FONT => match args.first().copied().unwrap_or("") {
            "16" => console::set_font_size(RasterHeight::Size16),
            "20" => console::set_font_size(RasterHeight::Size20),
            "24" => console::set_font_size(RasterHeight::Size24),
            "32" => console::set_font_size(RasterHeight::Size32),
            _ => kprintln!("usage: font <16|20|24|32>"),
        },
        Commands::CLEAR => console::reset(),
        Commands::SHUTDOWN => exit_qemu(0),
        Commands::EMPTY => {}
        Commands::UNKNOWN => kprintln!("Unknown command"),
    }
}

/// Prints help text for a specific command, or a full command listing if
/// no argument is given.
///
/// Usage: `help [command]`
fn help(args: &[&str]) {
    if let Some(cmd) = args.first() {
        match *cmd {
            "help" => {
                kprintln!("help - show available commands");
                kprintln!("usage: help [command]");
                kprintln!("example: help echo");
            }
            "echo" => {
                kprintln!("echo - print text to the screen");
                kprintln!("usage: echo <text>");
                kprintln!("example: echo hello world");
            }
            "clear" => {
                kprintln!("clear - clear the screen and reset cursor");
                kprintln!("usage: clear");
            }
            "about" => {
                kprintln!("about - show information about AgnostOS");
                kprintln!("usage: about");
            }
            "history" => {
                kprintln!("history - reprint visible screen history");
                kprintln!("usage: history");
            }
            "font" => {
                kprintln!("font - change the font size");
                kprintln!("usage: font <16|20|24|32>");
                kprintln!("example: font 24");
            }
            "meminfo" => {
                kprintln!("meminfo - show heap memory information");
                kprintln!("usage: meminfo");
            }
            _ => kprintln!("unknown command: {}", cmd),
        }
    } else {
        kprintln!("AgnostOS shell - available commands:");
        kprintln!("");
        kprintln!("  help      show this message, or help for a specific command");
        kprintln!("  echo      print text to the screen");
        kprintln!("  clear     clear the screen");
        kprintln!("  about     show OS information");
        kprintln!("  history   reprint screen history");
        kprintln!("  font      change font size");
        kprintln!("  meminfo   show heap memory information");
        kprintln!("  shutdown  shuts the machine down");
        kprintln!("");
        kprintln!("tip: type 'help <command>' for more details");
        kprintln!("tip: ctrl+c to cancel, ctrl+plus/minus to zoom");
    }
}

use x86_64::instructions::port::Port;

pub fn exit_qemu(code: u32) -> ! {
    unsafe {
        let mut port = Port::<u32>::new(0xf4);
        port.write(code);
    }

    loop {
        x86_64::instructions::hlt();
    }
}
