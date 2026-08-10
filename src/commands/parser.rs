use alloc::vec::Vec;

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
}
