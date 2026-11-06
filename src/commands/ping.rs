//! `ping` shell command.

use core::net::Ipv4Addr;

/// Report whether the requested destination can be used by the current stack.
pub(crate) fn ping(args: &[&str]) {
    let Some(destination) = args.first().copied() else {
        kprintln!("usage: ping <IPv4 address or hostname>");
        return;
    };
    if args.len() > 1 {
        kprintln!("usage: ping <IPv4 address or hostname>");
        return;
    }

    if destination.parse::<Ipv4Addr>().is_err() {
        kprintln!("ping: hostname lookup is unavailable (DNS is not implemented)");
        return;
    }

    kprintln!("ping: no network interface is configured");
}
