mod help;
mod shutdown;

use crate::{HEAP_SIZE, HEAP_START, commands::help::help, console, kprintln};
use alloc::vec::Vec;
use core::sync::atomic::Ordering;
use noto_sans_mono_bitmap::RasterHeight;

pub(crate) enum Commands {
    Help,
    About,
    History,
    Echo,
    Meminfo,
    Font,
    Clear,
    Shutdown,
    Empty,
    Unknown,
}

impl Commands {
    fn text(&self) -> &str {
        match self {
            Commands::Help => "help",
            Commands::About => "about",
            Commands::History => "history",
            Commands::Echo => "echo",
            Commands::Meminfo => "meminfo",
            Commands::Font => "font",
            Commands::Clear => "clear",
            Commands::Shutdown => "shutdown",
            Commands::Empty => "",
            Commands::Unknown => "unknown",
        }
    }
}

impl From<&str> for Commands {
    fn from(value: &str) -> Self {
        match value {
            "help" => Commands::Help,
            "about" => Commands::About,
            "history" => Commands::History,
            "echo" => Commands::Echo,
            "meminfo" => Commands::Meminfo,
            "font" => Commands::Font,
            "clear" => Commands::Clear,
            "shutdown" => Commands::Shutdown,
            "" => Commands::Empty,
            _ => Commands::Unknown,
        }
    }
}

/// Parses and dispatches a command string.
///
/// Splits the input into a command name, flags (tokens starting with `-`),
/// and positional arguments. Dispatches to the appropriate handler or prints
/// "Unknown command" if the command is not recognized.
pub(crate) fn run_command(command: &str) {
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
        Commands::Help => help(&args),
        Commands::About => {
            kprintln!("AgnostOS v0.1 - written in Rust \n codeberg.com/guru901/agnostos")
        }
        Commands::History => console::print_history(),
        Commands::Echo => kprintln!("{}", args.join(" ")),
        Commands::Meminfo => {
            let start = HEAP_START.load(Ordering::Relaxed);
            let size = HEAP_SIZE.load(Ordering::Relaxed);
            kprintln!("heap start: {:#x}", start);
            kprintln!("heap size:  {}mb", size / (1024 * 1024));
        }
        Commands::Font => match args.first().copied().unwrap_or("") {
            "16" => console::set_font_size(RasterHeight::Size16),
            "20" => console::set_font_size(RasterHeight::Size20),
            "24" => console::set_font_size(RasterHeight::Size24),
            "32" => console::set_font_size(RasterHeight::Size32),
            _ => kprintln!("usage: font <16|20|24|32>"),
        },
        Commands::Clear => console::reset(),
        Commands::Shutdown => shutdown::exit_qemu(0),
        Commands::Empty => {}
        Commands::Unknown => kprintln!("Unknown command"),
    }
}
