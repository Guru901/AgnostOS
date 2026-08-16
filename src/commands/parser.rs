use alloc::vec::Vec;

const COMMAND_NAMES: &[&str] = &[
    "about", "clear", "echo", "font", "help", "history", "meminfo", "shutdown", "uptime",
];

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum Command {
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
    Uptime,
}

/// Returns a command name only when `prefix` identifies exactly one command.
///
/// Completing ambiguous prefixes would make Tab choose an arbitrary command,
/// so those prefixes intentionally remain unchanged.
pub(crate) fn complete_command(prefix: &str) -> Option<&'static str> {
    if prefix.is_empty() || prefix.chars().any(char::is_whitespace) {
        return None;
    }

    let mut matches = COMMAND_NAMES
        .iter()
        .copied()
        .filter(|command| command.starts_with(prefix));
    let command = matches.next()?;

    matches.next().is_none().then_some(command)
}

pub(crate) struct ParsedCommand<'a> {
    pub(crate) command: Command,
    pub(crate) args: Vec<&'a str>,
    pub(crate) flags: Vec<&'a str>,
}

pub(crate) fn parse(input: &str) -> ParsedCommand<'_> {
    let mut tokens = input.trim().split_whitespace();
    let command = match tokens.next().unwrap_or("") {
        "help" => Command::Help,
        "about" => Command::About,
        "history" => Command::History,
        "echo" => Command::Echo,
        "meminfo" => Command::Meminfo,
        "font" => Command::Font,
        "clear" => Command::Clear,
        "shutdown" => Command::Shutdown,
        "uptime" => Command::Uptime,
        "" => Command::Empty,
        _ => Command::Unknown,
    };
    let mut args = Vec::new();
    let mut flags = Vec::new();
    for token in tokens {
        if token.starts_with('-') {
            flags.push(token);
        } else {
            args.push(token);
        }
    }
    ParsedCommand {
        command,
        args,
        flags,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_separates_command_arguments_and_flags() {
        let parsed = parse("font --temporary 24");
        assert_eq!(parsed.command, Command::Font);
        assert_eq!(parsed.flags.as_slice(), ["--temporary"]);
        assert_eq!(parsed.args.as_slice(), ["24"]);
    }

    #[test]
    fn parser_recognizes_uptime() {
        let parsed = parse("uptime");
        assert_eq!(parsed.command, Command::Uptime);
        assert!(parsed.args.is_empty());
        assert!(parsed.flags.is_empty());
    }

    #[test]
    fn autocomplete_returns_the_single_matching_command() {
        assert_eq!(complete_command("upt"), Some("uptime"));
    }

    #[test]
    fn autocomplete_leaves_ambiguous_or_non_command_input_alone() {
        assert_eq!(complete_command("h"), None);
        assert_eq!(complete_command("nope"), None);
        assert_eq!(complete_command("echo hello"), None);
    }
}
