//! Allocation-free building blocks for the kernel network stack.
//!
//! The first protocols are Ethernet, ARP for IPv4 over Ethernet, and IPv4.
//! The network path currently targets a polling RTL8139 device. QEMU user-mode
//! networking uses a fixed local address and gateway configured below.

use core::{convert::TryInto, net::Ipv4Addr};
use spin::Mutex;

mod pci;
mod rtl8139;

const LOCAL_IPV4: [u8; 4] = [10, 0, 2, 15];
const GATEWAY_IPV4: [u8; 4] = [10, 0, 2, 2];
const DNS_IPV4: [u8; 4] = [10, 0, 2, 3];
const SUBNET_MASK: [u8; 4] = [255, 255, 255, 0];
const PING_ID: u16 = 0x4147;
const MAX_FRAME_LEN: usize = 1536;
const ECHO_DATA: &[u8] = b"agnostos ping";
const DNS_PORT: u16 = 53000;
const DNS_QUERY_ID: u16 = 0x4e53;

#[derive(Clone, Copy)]
struct NetworkConfig {
    mac: MacAddress,
    learned_ip: [u8; 4],
    learned_mac: MacAddress,
}

static NETWORK: Mutex<Option<NetworkConfig>> = Mutex::new(None);

/// Why a network operation could not be completed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkError {
    /// No supported network interface was initialized.
    InterfaceUnavailable,
    /// The NIC or packet path failed while sending/receiving.
    DeviceFailure,
    /// No ARP response arrived for the next-hop address.
    AddressResolutionTimeout,
    /// No matching ICMP echo reply arrived before the timeout.
    EchoTimeout,
    /// The configured DNS server did not return an IPv4 answer.
    DnsFailure,
}

/// Initialize the polling NIC and the QEMU user-network interface defaults.
pub fn initialize() -> Result<MacAddress, NetworkError> {
    let mac = match rtl8139::initialize() {
        Ok(mac) => mac,
        Err(rtl8139::DriverError::NotFound) => return Err(NetworkError::InterfaceUnavailable),
        Err(_) => return Err(NetworkError::DeviceFailure),
    };
    *NETWORK.lock() = Some(NetworkConfig {
        mac,
        learned_ip: [0; 4],
        learned_mac: MacAddress::default(),
    });
    Ok(mac)
}

/// Send one ICMP echo request and wait for its matching reply.
pub fn ping(destination: Ipv4Addr, sequence: u16, timeout_ms: u64) -> Result<u64, NetworkError> {
    let destination = destination.octets();
    let config = NETWORK
        .lock()
        .as_ref()
        .copied()
        .ok_or(NetworkError::InterfaceUnavailable)?;
    let next_hop = if same_subnet(destination, LOCAL_IPV4) {
        destination
    } else {
        GATEWAY_IPV4
    };
    let destination_mac = resolve_address(next_hop, config)?;

    let mut icmp = [0u8; ICMPV4_ECHO_HEADER_LEN + ECHO_DATA.len()];
    let echo = Icmpv4Echo {
        message_type: 8,
        identifier: PING_ID,
        sequence,
        payload: ECHO_DATA,
    };
    let icmp_len = echo
        .write(&mut icmp)
        .map_err(|_| NetworkError::DeviceFailure)?;
    let mut ipv4 = [0u8; IPV4_MIN_HEADER_LEN + ICMPV4_ECHO_HEADER_LEN + ECHO_DATA.len()];
    let ipv4_len = Ipv4Packet::write(LOCAL_IPV4, destination, 64, 1, &icmp[..icmp_len], &mut ipv4)
        .map_err(|_| NetworkError::DeviceFailure)?;
    transmit_frame(
        config.mac,
        destination_mac,
        EtherType::Ipv4,
        &ipv4[..ipv4_len],
    )?;

    let started = crate::timer::uptime_ms();
    let deadline = started.saturating_add(timeout_ms.max(1));
    let mut frame_buffer = [0u8; MAX_FRAME_LEN];
    while crate::timer::uptime_ms() <= deadline {
        if let Some(NetEvent::EchoReply(source, reply_id, reply_sequence)) =
            receive_and_process(config, &mut frame_buffer)?
        {
            if source == destination && reply_id == PING_ID && reply_sequence == sequence {
                return Ok(crate::timer::uptime_ms().saturating_sub(started));
            }
        }
        core::hint::spin_loop();
    }
    Err(NetworkError::EchoTimeout)
}

/// Resolve a hostname through the DNS server supplied by QEMU user networking.
pub fn resolve_hostname(hostname: &str) -> Result<Ipv4Addr, NetworkError> {
    let config = NETWORK
        .lock()
        .as_ref()
        .copied()
        .ok_or(NetworkError::InterfaceUnavailable)?;
    let destination_mac = resolve_address(DNS_IPV4, config)?;

    let mut dns = [0u8; 512];
    let dns_len = build_dns_query(hostname, DNS_QUERY_ID, &mut dns)?;
    let mut udp = [0u8; 520];
    let udp_len = UdpDatagram::write(DNS_PORT, 53, &dns[..dns_len], &mut udp)
        .map_err(|_| NetworkError::DnsFailure)?;
    let mut ipv4 = [0u8; IPV4_MIN_HEADER_LEN + 520];
    let ipv4_len = Ipv4Packet::write(LOCAL_IPV4, DNS_IPV4, 64, 17, &udp[..udp_len], &mut ipv4)
        .map_err(|_| NetworkError::DnsFailure)?;
    transmit_frame(
        config.mac,
        destination_mac,
        EtherType::Ipv4,
        &ipv4[..ipv4_len],
    )?;

    let deadline = crate::timer::uptime_ms().saturating_add(3_000);
    let mut frame_buffer = [0u8; MAX_FRAME_LEN];
    while crate::timer::uptime_ms() <= deadline {
        if let Some(NetEvent::DnsReply(address)) = receive_and_process(config, &mut frame_buffer)? {
            return address.map(Ipv4Addr::from).ok_or(NetworkError::DnsFailure);
        }
        core::hint::spin_loop();
    }
    Err(NetworkError::DnsFailure)
}

fn build_dns_query(
    hostname: &str,
    transaction_id: u16,
    out: &mut [u8],
) -> Result<usize, NetworkError> {
    if hostname.is_empty() || hostname.len() > 253 || out.len() < 17 {
        return Err(NetworkError::DnsFailure);
    }
    out[..12].fill(0);
    out[0..2].copy_from_slice(&transaction_id.to_be_bytes());
    out[2..4].copy_from_slice(&0x0100u16.to_be_bytes()); // recursion desired
    out[4..6].copy_from_slice(&1u16.to_be_bytes()); // one question
    let mut cursor = 12;
    for label in hostname.split('.') {
        if label.is_empty() || label.len() > 63 || cursor + label.len() + 5 > out.len() {
            return Err(NetworkError::DnsFailure);
        }
        out[cursor] = label.len() as u8;
        cursor += 1;
        out[cursor..cursor + label.len()].copy_from_slice(label.as_bytes());
        cursor += label.len();
    }
    out[cursor] = 0;
    cursor += 1;
    out[cursor..cursor + 2].copy_from_slice(&1u16.to_be_bytes()); // A
    out[cursor + 2..cursor + 4].copy_from_slice(&1u16.to_be_bytes()); // IN
    Ok(cursor + 4)
}

fn parse_dns_a_response(bytes: &[u8], transaction_id: u16) -> Option<[u8; 4]> {
    if bytes.len() < 12
        || u16::from_be_bytes([bytes[0], bytes[1]]) != transaction_id
        || bytes[2] & 0x80 == 0
        || bytes[3] & 0x0f != 0
    {
        return None;
    }
    let questions = usize::from(u16::from_be_bytes([bytes[4], bytes[5]]));
    let answers = usize::from(u16::from_be_bytes([bytes[6], bytes[7]]));
    let mut cursor = 12;
    for _ in 0..questions {
        cursor = skip_dns_name(bytes, cursor)?;
        cursor = cursor.checked_add(4)?; // QTYPE + QCLASS
        if cursor > bytes.len() {
            return None;
        }
    }
    for _ in 0..answers {
        cursor = skip_dns_name(bytes, cursor)?;
        if cursor + 10 > bytes.len() {
            return None;
        }
        let kind = u16::from_be_bytes([bytes[cursor], bytes[cursor + 1]]);
        let class = u16::from_be_bytes([bytes[cursor + 2], bytes[cursor + 3]]);
        let length = usize::from(u16::from_be_bytes([bytes[cursor + 8], bytes[cursor + 9]]));
        cursor += 10;
        if cursor + length > bytes.len() {
            return None;
        }
        if kind == 1 && class == 1 && length == 4 {
            return Some(bytes[cursor..cursor + 4].try_into().ok()?);
        }
        cursor += length;
    }
    None
}

fn skip_dns_name(bytes: &[u8], mut cursor: usize) -> Option<usize> {
    loop {
        let length = usize::from(*bytes.get(cursor)?);
        if length & 0xc0 == 0xc0 {
            if bytes.get(cursor + 1).is_none() {
                return None;
            }
            return Some(cursor + 2);
        }
        if length & 0xc0 != 0 {
            return None;
        }
        cursor += 1;
        if length == 0 {
            return Some(cursor);
        }
        cursor = cursor.checked_add(length)?;
        if cursor > bytes.len() {
            return None;
        }
    }
}

fn same_subnet(left: [u8; 4], right: [u8; 4]) -> bool {
    (0..4).all(|i| left[i] & SUBNET_MASK[i] == right[i] & SUBNET_MASK[i])
}

fn resolve_address(address: [u8; 4], config: NetworkConfig) -> Result<MacAddress, NetworkError> {
    {
        let state = NETWORK.lock();
        if let Some(state) = *state
            && state.learned_ip == address
        {
            return Ok(state.learned_mac);
        }
    }

    let request = ArpPacket {
        operation: ArpOperation::Request,
        sender_hardware: config.mac,
        sender_protocol: LOCAL_IPV4,
        target_hardware: MacAddress::default(),
        target_protocol: address,
    };
    let mut arp = [0u8; ARP_PACKET_LEN];
    request
        .write(&mut arp)
        .map_err(|_| NetworkError::DeviceFailure)?;
    transmit_frame(config.mac, ETHERNET_BROADCAST, EtherType::Arp, &arp)?;

    let deadline = crate::timer::uptime_ms().saturating_add(1_000);
    let mut frame_buffer = [0u8; MAX_FRAME_LEN];
    while crate::timer::uptime_ms() <= deadline {
        let _ = receive_and_process(config, &mut frame_buffer)?;
        let state = NETWORK.lock();
        if let Some(state) = *state
            && state.learned_ip == address
        {
            return Ok(state.learned_mac);
        }
        core::hint::spin_loop();
    }
    Err(NetworkError::AddressResolutionTimeout)
}

fn transmit_frame(
    source: MacAddress,
    destination: MacAddress,
    ether_type: EtherType,
    payload: &[u8],
) -> Result<(), NetworkError> {
    let mut frame = [0u8; MAX_FRAME_LEN];
    let frame_packet = EthernetFrame {
        destination,
        source,
        ether_type,
        payload,
    };
    let length = frame_packet
        .write(&mut frame)
        .map_err(|_| NetworkError::DeviceFailure)?;
    let length = length.max(60);
    rtl8139::transmit(&frame[..length]).map_err(|_| NetworkError::DeviceFailure)
}

fn receive_and_process(
    config: NetworkConfig,
    frame_buffer: &mut [u8; MAX_FRAME_LEN],
) -> Result<Option<NetEvent>, NetworkError> {
    let Some(length) = rtl8139::receive(frame_buffer).map_err(|_| NetworkError::DeviceFailure)?
    else {
        return Ok(None);
    };
    let Ok(frame) = EthernetFrame::parse(&frame_buffer[..length]) else {
        return Ok(None);
    };
    match frame.ether_type {
        EtherType::Arp => {
            if let Ok(packet) = ArpPacket::parse(frame.payload) {
                if packet.sender_protocol != [0; 4] {
                    let mut state = NETWORK.lock();
                    if let Some(state) = state.as_mut() {
                        state.learned_ip = packet.sender_protocol;
                        state.learned_mac = packet.sender_hardware;
                    }
                }
                if packet.operation == ArpOperation::Request && packet.target_protocol == LOCAL_IPV4
                {
                    let reply = ArpPacket {
                        operation: ArpOperation::Reply,
                        sender_hardware: config.mac,
                        sender_protocol: LOCAL_IPV4,
                        target_hardware: packet.sender_hardware,
                        target_protocol: packet.sender_protocol,
                    };
                    let mut bytes = [0u8; ARP_PACKET_LEN];
                    reply
                        .write(&mut bytes)
                        .map_err(|_| NetworkError::DeviceFailure)?;
                    transmit_frame(config.mac, packet.sender_hardware, EtherType::Arp, &bytes)?;
                }
            }
        }
        EtherType::Ipv4 => {
            let Ok(packet) = Ipv4Packet::parse(frame.payload) else {
                return Ok(None);
            };
            if packet.destination != LOCAL_IPV4 {
                return Ok(None);
            }
            match packet.protocol {
                1 => {
                    if let Ok(echo) = Icmpv4Echo::parse(packet.payload)
                        && echo.message_type == 0
                    {
                        return Ok(Some(NetEvent::EchoReply(
                            packet.source,
                            echo.identifier,
                            echo.sequence,
                        )));
                    }
                }
                17 => {
                    if let Ok(udp) = UdpDatagram::parse(packet.payload)
                        && udp.destination_port == DNS_PORT
                    {
                        return Ok(Some(NetEvent::DnsReply(parse_dns_a_response(
                            udp.payload,
                            DNS_QUERY_ID,
                        ))));
                    }
                }
                _ => {}
            }
        }
        EtherType::Other(_) => {}
    }
    Ok(None)
}

#[derive(Clone, Copy)]
enum NetEvent {
    EchoReply([u8; 4], u16, u16),
    DnsReply(Option<[u8; 4]>),
}

/// The Ethernet broadcast address.
pub const ETHERNET_BROADCAST: MacAddress = MacAddress([0xff; 6]);
/// Size of an Ethernet header without a VLAN tag.
pub const ETHERNET_HEADER_LEN: usize = 14;
/// Size of the fixed Ethernet/IPv4 ARP packet.
pub const ARP_PACKET_LEN: usize = 28;
/// Size of the fixed portion of an IPv4 header.
pub const IPV4_MIN_HEADER_LEN: usize = 20;
/// ICMPv4 echo message header length.
pub const ICMPV4_ECHO_HEADER_LEN: usize = 8;

/// A 48-bit Ethernet hardware address.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct MacAddress(pub [u8; 6]);

impl MacAddress {
    /// Construct an address from its six octets.
    #[must_use]
    pub const fn new(octets: [u8; 6]) -> Self {
        Self(octets)
    }

    /// Return the address octets.
    #[must_use]
    pub const fn octets(self) -> [u8; 6] {
        self.0
    }
}

/// Ethernet frame protocol identifiers supported by this module.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EtherType {
    /// Address Resolution Protocol.
    Arp,
    /// Internet Protocol version 4.
    Ipv4,
    /// An EtherType this stack does not currently handle.
    Other(u16),
}

impl EtherType {
    const fn from_u16(value: u16) -> Self {
        match value {
            0x0806 => Self::Arp,
            0x0800 => Self::Ipv4,
            other => Self::Other(other),
        }
    }

    const fn as_u16(self) -> u16 {
        match self {
            Self::Arp => 0x0806,
            Self::Ipv4 => 0x0800,
            Self::Other(value) => value,
        }
    }
}

/// A borrowed Ethernet frame. The payload excludes the 14-byte header.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EthernetFrame<'a> {
    /// Destination hardware address.
    pub destination: MacAddress,
    /// Source hardware address.
    pub source: MacAddress,
    /// Protocol carried by the payload.
    pub ether_type: EtherType,
    /// Frame payload.
    pub payload: &'a [u8],
}

impl<'a> EthernetFrame<'a> {
    /// Parse an Ethernet II frame from bytes.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, PacketError> {
        if bytes.len() < ETHERNET_HEADER_LEN {
            return Err(PacketError::Truncated);
        }
        Ok(Self {
            destination: MacAddress(bytes[0..6].try_into().unwrap()),
            source: MacAddress(bytes[6..12].try_into().unwrap()),
            ether_type: EtherType::from_u16(u16::from_be_bytes([bytes[12], bytes[13]])),
            payload: &bytes[ETHERNET_HEADER_LEN..],
        })
    }

    /// Write the frame into `out`, returning the number of bytes written.
    pub fn write(&self, out: &mut [u8]) -> Result<usize, PacketError> {
        let total = ETHERNET_HEADER_LEN
            .checked_add(self.payload.len())
            .ok_or(PacketError::BufferTooSmall)?;
        if out.len() < total {
            return Err(PacketError::BufferTooSmall);
        }
        out[..6].copy_from_slice(&self.destination.0);
        out[6..12].copy_from_slice(&self.source.0);
        out[12..14].copy_from_slice(&self.ether_type.as_u16().to_be_bytes());
        out[ETHERNET_HEADER_LEN..total].copy_from_slice(self.payload);
        Ok(total)
    }
}

/// Address Resolution Protocol operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArpOperation {
    /// Request a hardware address for a protocol address.
    Request,
    /// Reply with a hardware address.
    Reply,
    /// An operation this implementation does not handle.
    Other(u16),
}

impl ArpOperation {
    const fn from_u16(value: u16) -> Self {
        match value {
            1 => Self::Request,
            2 => Self::Reply,
            other => Self::Other(other),
        }
    }

    const fn as_u16(self) -> u16 {
        match self {
            Self::Request => 1,
            Self::Reply => 2,
            Self::Other(value) => value,
        }
    }
}

/// ARP packet for IPv4 addresses carried over Ethernet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArpPacket {
    /// Request or reply operation.
    pub operation: ArpOperation,
    /// Hardware address of the sender.
    pub sender_hardware: MacAddress,
    /// IPv4 address of the sender.
    pub sender_protocol: [u8; 4],
    /// Hardware address being queried (zero for an unresolved request).
    pub target_hardware: MacAddress,
    /// IPv4 address being queried.
    pub target_protocol: [u8; 4],
}

impl ArpPacket {
    /// Parse a standard Ethernet/IPv4 ARP packet.
    pub fn parse(bytes: &[u8]) -> Result<Self, PacketError> {
        if bytes.len() < ARP_PACKET_LEN {
            return Err(PacketError::Truncated);
        }
        if u16::from_be_bytes([bytes[0], bytes[1]]) != 1
            || u16::from_be_bytes([bytes[2], bytes[3]]) != 0x0800
            || bytes[4] != 6
            || bytes[5] != 4
        {
            return Err(PacketError::UnsupportedFormat);
        }
        Ok(Self {
            operation: ArpOperation::from_u16(u16::from_be_bytes([bytes[6], bytes[7]])),
            sender_hardware: MacAddress(bytes[8..14].try_into().unwrap()),
            sender_protocol: bytes[14..18].try_into().unwrap(),
            target_hardware: MacAddress(bytes[18..24].try_into().unwrap()),
            target_protocol: bytes[24..28].try_into().unwrap(),
        })
    }

    /// Write this standard Ethernet/IPv4 ARP packet to `out`.
    pub fn write(&self, out: &mut [u8]) -> Result<usize, PacketError> {
        if out.len() < ARP_PACKET_LEN {
            return Err(PacketError::BufferTooSmall);
        }
        out[..ARP_PACKET_LEN].fill(0);
        out[0..2].copy_from_slice(&1u16.to_be_bytes());
        out[2..4].copy_from_slice(&0x0800u16.to_be_bytes());
        out[4] = 6;
        out[5] = 4;
        out[6..8].copy_from_slice(&self.operation.as_u16().to_be_bytes());
        out[8..14].copy_from_slice(&self.sender_hardware.0);
        out[14..18].copy_from_slice(&self.sender_protocol);
        out[18..24].copy_from_slice(&self.target_hardware.0);
        out[24..28].copy_from_slice(&self.target_protocol);
        Ok(ARP_PACKET_LEN)
    }
}

/// A borrowed IPv4 packet with a validated header checksum and length.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Ipv4Packet<'a> {
    /// Header length in bytes, including options.
    pub header_len: usize,
    /// Total packet length in bytes.
    pub total_len: usize,
    /// Time-to-live value.
    pub ttl: u8,
    /// Upper-layer protocol number (for example 1=ICMP, 6=TCP, 17=UDP).
    pub protocol: u8,
    /// Source IPv4 address in network byte order.
    pub source: [u8; 4],
    /// Destination IPv4 address in network byte order.
    pub destination: [u8; 4],
    /// IPv4 options, if present.
    pub options: &'a [u8],
    /// Encapsulated payload, excluding the IPv4 header.
    pub payload: &'a [u8],
}

impl<'a> Ipv4Packet<'a> {
    /// Parse and validate an IPv4 packet, including header checksum.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, PacketError> {
        if bytes.len() < IPV4_MIN_HEADER_LEN {
            return Err(PacketError::Truncated);
        }
        if bytes[0] >> 4 != 4 {
            return Err(PacketError::UnsupportedFormat);
        }
        let header_len = usize::from(bytes[0] & 0x0f) * 4;
        if header_len < IPV4_MIN_HEADER_LEN || bytes.len() < header_len {
            return Err(PacketError::Truncated);
        }
        let total_len = usize::from(u16::from_be_bytes([bytes[2], bytes[3]]));
        if total_len < header_len || bytes.len() < total_len {
            return Err(PacketError::Truncated);
        }
        if internet_checksum(&bytes[..header_len]) != 0 {
            return Err(PacketError::InvalidChecksum);
        }
        Ok(Self {
            header_len,
            total_len,
            ttl: bytes[8],
            protocol: bytes[9],
            source: bytes[12..16].try_into().unwrap(),
            destination: bytes[16..20].try_into().unwrap(),
            options: &bytes[IPV4_MIN_HEADER_LEN..header_len],
            payload: &bytes[header_len..total_len],
        })
    }

    /// Write an IPv4 header and payload, computing its header checksum.
    /// Fragmentation fields are zeroed because fragmentation is not supported.
    pub fn write(
        source: [u8; 4],
        destination: [u8; 4],
        ttl: u8,
        protocol: u8,
        payload: &[u8],
        out: &mut [u8],
    ) -> Result<usize, PacketError> {
        let total_len = IPV4_MIN_HEADER_LEN
            .checked_add(payload.len())
            .filter(|&len| len <= usize::from(u16::MAX))
            .ok_or(PacketError::BufferTooSmall)?;
        if out.len() < total_len {
            return Err(PacketError::BufferTooSmall);
        }
        out[..IPV4_MIN_HEADER_LEN].fill(0);
        out[0] = 0x45;
        out[2..4].copy_from_slice(&(total_len as u16).to_be_bytes());
        out[8] = ttl;
        out[9] = protocol;
        out[12..16].copy_from_slice(&source);
        out[16..20].copy_from_slice(&destination);
        let checksum = internet_checksum(&out[..IPV4_MIN_HEADER_LEN]);
        out[10..12].copy_from_slice(&checksum.to_be_bytes());
        out[IPV4_MIN_HEADER_LEN..total_len].copy_from_slice(payload);
        Ok(total_len)
    }
}

/// Compute the Internet checksum (RFC 1071) over a byte slice.
#[must_use]
pub fn internet_checksum(bytes: &[u8]) -> u16 {
    let mut sum = 0u32;
    let mut chunks = bytes.chunks_exact(2);
    for pair in &mut chunks {
        sum += u32::from(u16::from_be_bytes([pair[0], pair[1]]));
    }
    if let Some(&last) = chunks.remainder().first() {
        sum += u32::from(last) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

/// ICMPv4 echo request or reply message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Icmpv4Echo<'a> {
    /// `8` for a request, `0` for a reply.
    pub message_type: u8,
    /// Echo identifier used to match requests and replies.
    pub identifier: u16,
    /// Sequence number used to match individual probes.
    pub sequence: u16,
    /// Echo data.
    pub payload: &'a [u8],
}

/// Borrowed UDP datagram with ports and a validated declared length.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UdpDatagram<'a> {
    /// Sender port.
    pub source_port: u16,
    /// Recipient port.
    pub destination_port: u16,
    /// UDP payload.
    pub payload: &'a [u8],
}

impl<'a> UdpDatagram<'a> {
    /// Parse a UDP datagram, allowing an IPv4 zero checksum.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, PacketError> {
        if bytes.len() < 8 {
            return Err(PacketError::Truncated);
        }
        let length = usize::from(u16::from_be_bytes([bytes[4], bytes[5]]));
        if length < 8 || bytes.len() < length {
            return Err(PacketError::Truncated);
        }
        Ok(Self {
            source_port: u16::from_be_bytes([bytes[0], bytes[1]]),
            destination_port: u16::from_be_bytes([bytes[2], bytes[3]]),
            payload: &bytes[8..length],
        })
    }

    /// Serialize a UDP datagram using the permitted zero IPv4 checksum.
    pub fn write(
        source_port: u16,
        destination_port: u16,
        payload: &[u8],
        out: &mut [u8],
    ) -> Result<usize, PacketError> {
        let length = 8usize
            .checked_add(payload.len())
            .filter(|&length| length <= usize::from(u16::MAX))
            .ok_or(PacketError::BufferTooSmall)?;
        if out.len() < length {
            return Err(PacketError::BufferTooSmall);
        }
        out[..8].fill(0);
        out[0..2].copy_from_slice(&source_port.to_be_bytes());
        out[2..4].copy_from_slice(&destination_port.to_be_bytes());
        out[4..6].copy_from_slice(&(length as u16).to_be_bytes());
        out[8..length].copy_from_slice(payload);
        Ok(length)
    }
}

impl<'a> Icmpv4Echo<'a> {
    /// Parse an ICMPv4 echo request or reply and validate its checksum.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, PacketError> {
        if bytes.len() < ICMPV4_ECHO_HEADER_LEN {
            return Err(PacketError::Truncated);
        }
        if !matches!(bytes[0], 0 | 8) || bytes[1] != 0 {
            return Err(PacketError::UnsupportedFormat);
        }
        if internet_checksum(bytes) != 0 {
            return Err(PacketError::InvalidChecksum);
        }
        Ok(Self {
            message_type: bytes[0],
            identifier: u16::from_be_bytes([bytes[4], bytes[5]]),
            sequence: u16::from_be_bytes([bytes[6], bytes[7]]),
            payload: &bytes[ICMPV4_ECHO_HEADER_LEN..],
        })
    }

    /// Serialize an echo request (`message_type` 8) or reply (`0`).
    pub fn write(&self, out: &mut [u8]) -> Result<usize, PacketError> {
        let total_len = ICMPV4_ECHO_HEADER_LEN
            .checked_add(self.payload.len())
            .ok_or(PacketError::BufferTooSmall)?;
        if out.len() < total_len {
            return Err(PacketError::BufferTooSmall);
        }
        if !matches!(self.message_type, 0 | 8) {
            return Err(PacketError::UnsupportedFormat);
        }
        out[..ICMPV4_ECHO_HEADER_LEN].fill(0);
        out[0] = self.message_type;
        out[4..6].copy_from_slice(&self.identifier.to_be_bytes());
        out[6..8].copy_from_slice(&self.sequence.to_be_bytes());
        out[ICMPV4_ECHO_HEADER_LEN..total_len].copy_from_slice(self.payload);
        let checksum = internet_checksum(&out[..total_len]);
        out[2..4].copy_from_slice(&checksum.to_be_bytes());
        Ok(total_len)
    }
}

/// Errors returned when packet bytes are malformed or do not fit a buffer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PacketError {
    /// The input ended before the packet or its declared length.
    Truncated,
    /// The packet uses a hardware or protocol format not supported here.
    UnsupportedFormat,
    /// A required checksum did not validate.
    InvalidChecksum,
    /// The output buffer cannot contain the serialized packet.
    BufferTooSmall,
}
