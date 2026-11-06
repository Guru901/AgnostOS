//! Allocation-free building blocks for the kernel network stack.
//!
//! The first protocols are Ethernet, ARP for IPv4 over Ethernet, and IPv4.
//! This module deliberately stops at packet handling: a network device driver
//! can hand received frames to these types and transmit the resulting bytes.

use core::convert::TryInto;

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
