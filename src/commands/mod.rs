//! Shell command parse-and-dispatch.
//!
//! Tokenization is whitespace-only. After the command token, tokens starting
//! with `-` are flags; the rest are positional arguments. Unrecognized names become
//! [`parser::Command::Unknown`].

mod help;
mod parser;
mod shutdown;

use crate::{commands::help::help, console, kprintln, memory, timer};
use noto_sans_mono_bitmap::RasterHeight;
use parser::{Command, parse};

/// Parses and dispatches a command string.
///
/// Splits the input into a command name, flags (tokens starting with `-`),
/// and positional arguments. Dispatches to the appropriate handler or prints
/// "Unknown command" if the command is not recognized.
pub(crate) fn run_command(command: &str) {
    let parsed = parse(command);
    let _ = parsed.flags;
    let args = parsed.args;

    match parsed.command {
        Command::Help => help(&args),
        Command::About => {
            kprintln!("AgnostOS v0.1 - written in Rust \n codeberg.com/guru901/agnostos");
        }
        Command::History => console::print_history(),
        Command::Echo => kprintln!("{}", args.join(" ")),
        Command::Meminfo => memory::MemorySummary::meminfo(),
        Command::Font => match args.first().copied().unwrap_or("") {
            "16" => console::set_font_size(RasterHeight::Size16),
            "20" => console::set_font_size(RasterHeight::Size20),
            "24" => console::set_font_size(RasterHeight::Size24),
            "32" => console::set_font_size(RasterHeight::Size32),
            _ => kprintln!("usage: font <16|20|24|32>"),
        },
        Command::Clear => console::reset(),
        Command::Shutdown => shutdown::exit_qemu(shutdown::QemuExitCode::SUCCESS),
        Command::Uptime => kprintln!("{}", timer::uptime()),
        Command::Empty => {}
        Command::Unknown => kprintln!("Unknown command"),
    }
}

/// Forwards Tab completion to the parser (unique prefix only).
pub(crate) fn complete_command(prefix: &str) -> Option<&'static str> {
    parser::complete_command(prefix)
}
