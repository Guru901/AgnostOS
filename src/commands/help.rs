use crate::kprintln;

use super::parser::{Command, parse};

/// Prints help text for a specific command, or a full command listing if
/// no argument is given.
///
/// Usage: `help [command]`
pub(crate) fn help(args: &[&str]) {
    if let Some(cmd) = args.first() {
        let command = parse(cmd).command;
        match command {
            Command::Help => {
                kprintln!("help - show available commands");
                kprintln!("usage: help [command]");
                kprintln!("example: help echo");
            }
            Command::Echo => {
                kprintln!("echo - print text to the screen");
                kprintln!("usage: echo <text>");
                kprintln!("example: echo hello world");
            }
            Command::Clear => {
                kprintln!("clear - clear the screen and reset cursor");
                kprintln!("usage: clear");
            }
            Command::About => {
                kprintln!("about - show information about AgnostOS");
                kprintln!("usage: about");
            }
            Command::History => {
                kprintln!("history - reprint visible screen history");
                kprintln!("usage: history");
            }
            Command::Font => {
                kprintln!("font - change the font size");
                kprintln!("usage: font <16|20|24|32>");
                kprintln!("example: font 24");
            }
            Command::Meminfo => {
                kprintln!("meminfo - show heap memory information");
                kprintln!("usage: meminfo");
            }
            Command::Uptime => {
                kprintln!("uptime - tells how long the system has been running for");
                kprintln!("usage: uptime");
            }
            Command::Shutdown => {
                kprintln!("shutdown - shuts the machine down instantly");
                kprintln!("usage: shutdown");
            }
            Command::Unknown => kprintln!("unknown command: {}", cmd),
            Command::Empty => {}
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
        kprintln!("  uptime    tells how long the system has been running for");
        kprintln!("  shutdown  shuts the machine down");
        kprintln!("");
        kprintln!("tip: type 'help <command>' for more details");
        kprintln!("tip: ctrl+c to cancel, ctrl+plus/minus to zoom");
    }
}
