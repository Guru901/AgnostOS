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

    let address = match destination.parse::<Ipv4Addr>() {
        Ok(address) => address,
        Err(_) => match crate::net::resolve_hostname(destination) {
            Ok(address) => address,
            Err(crate::net::NetworkError::InterfaceUnavailable) => {
                kprintln!("ping: no network interface is configured");
                return;
            }
            Err(crate::net::NetworkError::AddressResolutionTimeout) => {
                kprintln!("ping: could not resolve the DNS server with ARP");
                return;
            }
            Err(_) => {
                kprintln!("ping: DNS lookup failed for {}", destination);
                return;
            }
        },
    };
    kprintln!("PING {} ({}): {} data bytes", destination, address, 13);
    for sequence in 1..=4 {
        match crate::net::ping(address, sequence, 1_000) {
            Ok(elapsed_ms) => kprintln!(
                "21 bytes from {}: icmp_seq={} time={} ms",
                address,
                sequence,
                elapsed_ms
            ),
            Err(crate::net::NetworkError::InterfaceUnavailable) => {
                kprintln!("ping: no network interface is configured");
                return;
            }
            Err(crate::net::NetworkError::AddressResolutionTimeout) => {
                kprintln!("ping: ARP resolution timed out");
                return;
            }
            Err(crate::net::NetworkError::EchoTimeout) => {
                kprintln!("Request timeout for icmp_seq {}", sequence);
            }
            Err(crate::net::NetworkError::DeviceFailure) => {
                kprintln!("ping: network device error");
                return;
            }
            Err(crate::net::NetworkError::DnsFailure) => {
                kprintln!("ping: DNS lookup failed");
                return;
            }
        }
        if sequence != 4 {
            crate::timer::sleep_ms(1_000);
        }
    }
}
