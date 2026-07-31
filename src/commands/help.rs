use crate::kprintln;

/// Prints help text for a specific command, or a full command listing if
/// no argument is given.
///
/// Usage: `help [command]`
pub(crate) fn help(args: &[&str]) {
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
