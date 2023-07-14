extern crate base64;
use base64::{engine::general_purpose, Engine as _};
use bytes::{BufMut, Bytes, BytesMut};
use serde::{Deserialize, Serialize};
use std::char;
use std::convert::TryFrom;
use std::convert::TryInto;
use std::fmt::{Display, Formatter, Result as FmtResult, Write};
use std::ops::Index;
use std::str::from_utf8;

use crate::error::{Error, Result};
/// Enumeration of the `engine.io` `Packet` types.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum PacketId {
    Open,
    Close,
    Ping,
    Pong,
    Message,
    Upgrade,
    Noop,
}

impl PacketId {
    /// Returns the byte that represents the [`PacketId`] as a [`char`].
    fn to_string_byte(self) -> u8 {
        u8::from(self) + b'0'
    }
}

impl Display for PacketId {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.write_char(self.to_string_byte() as char)
    }
}

impl From<PacketId> for u8 {
    fn from(packet_id: PacketId) -> Self {
        match packet_id {
            PacketId::Open => 0,
            PacketId::Close => 1,
            PacketId::Ping => 2,
            PacketId::Pong => 3,
            PacketId::Message => 4,
            PacketId::Upgrade => 5,
            PacketId::Noop => 6,
        }
    }
}

impl TryFrom<u8> for PacketId {
    type Error = Error;
    /// Converts a byte into the corresponding `PacketId`.
    fn try_from(b: u8) -> Result<PacketId> {
        match b {
            0 | b'0' => Ok(PacketId::Open),
            1 | b'1' => Ok(PacketId::Close),
            2 | b'2' => Ok(PacketId::Ping),
            3 | b'3' => Ok(PacketId::Pong),
            4 | b'4' => Ok(PacketId::Message),
            5 | b'5' => Ok(PacketId::Upgrade),
            6 | b'6' => Ok(PacketId::Noop),
            _ => Err(Error::InvalidPacketId(b)),
        }
    }
}

/// A `Packet` sent via the `engine.io` protocol.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Packet {
    pub packet_id: PacketId,
    pub data: Bytes,
}

/// Data which gets exchanged in a handshake as defined by the server.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct HandshakePacket {
    pub sid: String,
    pub upgrades: Vec<String>,
    #[serde(rename = "pingInterval")]
    pub ping_interval: u64,
    #[serde(rename = "pingTimeout")]
    pub ping_timeout: u64,
}

impl TryFrom<Packet> for HandshakePacket {
    type Error = Error;
    fn try_from(packet: Packet) -> Result<HandshakePacket> {
        Ok(serde_json::from_slice(packet.data[..].as_ref())?)
    }
}

impl Packet {
    /// Creates a new `Packet`.
    pub fn new<T: Into<Bytes>>(packet_id: PacketId, data: T) -> Self {
        Packet {
            packet_id,
            data: data.into(),
        }
    }
}

impl From<Packet> for Bytes {
    fn from(packet: Packet) -> Self {
        let mut result = BytesMut::with_capacity(packet.data.len() + 1);
        result.put_u8(packet.packet_id.to_string_byte());
        result.put(packet.data);
        result.freeze()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct FramePayload(Packet);

#[derive(Debug, Clone)]
pub(crate) struct StrPayload(Vec<Packet>);

#[derive(Debug, Clone)]
pub(crate) struct BinPayload(Vec<Packet>); // TODO

// 4HelloWorld
// 2probe
impl TryFrom<Bytes> for FramePayload {
    type Error = Error;
    fn try_from(bytes: Bytes) -> Result<Self> {
        let packet_id = (*bytes.first().ok_or(Error::IncompletePacket())?).try_into()?;
        let data = bytes.slice(1..);

        let packet = Packet { packet_id, data };
        Ok(Self(packet))
    }
}

impl TryFrom<FramePayload> for Bytes {
    type Error = Error;
    fn try_from(payload: FramePayload) -> Result<Self> {
        payload.0.try_into()
    }
}

// 6:4hello2:4€
// 2:4€10:b4AQIDBA==
impl TryFrom<Bytes> for StrPayload {
    type Error = Error;
    fn try_from(bytes: Bytes) -> Result<Self> {
        let str = from_utf8(bytes.as_ref())?;
        let mut chars = str.chars();

        let mut packets: Vec<Packet> = Vec::new();
        let mut is_bin = false;

        loop {
            let mut cnt = 0;
            loop {
                match chars.next() {
                    Some(c) => {
                        match c {
                            '0'..='9' => cnt += cnt * 10 + c - '0',
                            ':' => break,
                            _ => return Err(Error::IncompletePacket()),
                        }
                    }
                    None => return Err(Error::IncompletePacket())
                }
            }
            if cnt == 0 {
                return Err(Error::IncompletePacket())
            }

            let packet_id = match chars.next() {
                Some(c) => {
                    match c {
                        '0'..='9' => {
                            cnt -= 1;
                            PacketId::try_from(c - '0')?
                        }
                        'b' => {
                            cnt -= 2;
                            is_bin = true;
                            PacketId::try_from(chars.next() - '0')?
                        }
                        _ => return Err(Error::IncompletePacket())
                    }
                }
                None => return Err(Error::IncompletePacket())
            };
            let mut str = "";
            for _ in 0..cnt {
                str += chars.next().ok_or(Error::IncompletePacket())?
            }

            packets.push(Packet {
                packet_id,
                data: if is_bin {
                    Bytes::from(general_purpose::STANDARD.decode(str))
                } else {
                    Bytes::from(str)
                },
            });

            if chars.clone().next().is_none() {
                break;
            }
        }

        Ok(Self(packets))
    }
}

impl TryFrom<StrPayload> for Bytes {
    type Error = Error;
    fn try_from(packets: StrPayload) -> Result<Self> {
        // TODO
    }
}

/**
  * 0                    => string
  * 6                    => byte length
  * 255                  => delimiter
  * 52                   => 4 (MESSAGE packet type)
  * 104 101 108 108 111  => "hello"
  * 1                    => binary
  * 5                    => byte length
  * 255                  => delimiter
  * 4                    => 4 (MESSAGE packet type)
  * 1 2 3 4              => binary message
  * Uint8Array.from([0, 6, 255, 52, 104, 101, 108, 108, 111, 1, 5, 255, 4, 1, 2, 3, 4]).buffer;
 */
impl TryFrom(Bytes) for BinPayload {
    type Error = Error;
    fn try_from(payload: Bytes) -> Result<Self> {
        // TODO
    }
}

impl TryFrom(BinPayload) for Bytes {
    type Error = Error;
    fn try_from(packets: BinPayload) -> Result<Self> {
        // TODO
    }
}

#[derive(Clone, Debug)]
pub struct IntoIter {
    iter: std::vec::IntoIter<Packet>,
}

impl Iterator for IntoIter {
    type Item = Packet;
    fn next(&mut self) -> std::option::Option<<Self as std::iter::Iterator>::Item> {
        self.iter.next()
    }
}

impl IntoIterator for StrPayload {
    type Item = Packet;
    type IntoIter = IntoIter;
    fn into_iter(self) -> <Self as std::iter::IntoIterator>::IntoIter {
        IntoIter {
            iter: self.0.into_iter(),
        }
    }
}

impl Index<usize> for StrPayload {
    type Output = Packet;
    fn index(&self, index: usize) -> &Packet {
        &self.0[index]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_packet_error() {
        let err = Packet::try_from(BytesMut::with_capacity(10).freeze());
        assert!(err.is_err())
    }

    #[test]
    fn test_is_reflexive() {
        let data = Bytes::from_static(b"1Hello World");
        let packet = Packet::try_from(data).unwrap();

        assert_eq!(packet.packet_id, PacketId::Close);
        assert_eq!(packet.data, Bytes::from_static(b"Hello World"));

        let data = Bytes::from_static(b"1Hello World");
        assert_eq!(Bytes::from(packet), data);
    }

    #[test]
    fn test_binary_packet() {
        // SGVsbG8= is the encoded string for 'Hello'
        let data = Bytes::from_static(b"bSGVsbG8=");
        let packet = Packet::try_from(data.clone()).unwrap();

        assert_eq!(packet.packet_id, PacketId::MessageBinary);
        assert_eq!(packet.data, Bytes::from_static(b"Hello"));

        assert_eq!(Bytes::from(packet), data);
    }

    #[test]
    fn test_decode_payload() -> Result<()> {
        let data = Bytes::from_static(b"1Hello\x1e1HelloWorld");
        let packets = StrPayload::try_from(data)?;

        assert_eq!(packets[0].packet_id, PacketId::Close);
        assert_eq!(packets[0].data, Bytes::from_static(b"Hello"));
        assert_eq!(packets[1].packet_id, PacketId::Close);
        assert_eq!(packets[1].data, Bytes::from_static(b"HelloWorld"));

        let data = "1Hello\x1e1HelloWorld".to_owned().into_bytes();
        assert_eq!(Bytes::try_from(packets).unwrap(), data);

        Ok(())
    }

    #[test]
    fn test_binary_payload() {
        let data = Bytes::from_static(b"bSGVsbG8=\x1ebSGVsbG9Xb3JsZA==\x1ebSGVsbG8=");
        let packets = StrPayload::try_from(data.clone()).unwrap();

        assert!(packets.0.len() == 3);
        assert_eq!(packets[0].packet_id, PacketId::MessageBinary);
        assert_eq!(packets[0].data, Bytes::from_static(b"Hello"));
        assert_eq!(packets[1].packet_id, PacketId::MessageBinary);
        assert_eq!(packets[1].data, Bytes::from_static(b"HelloWorld"));
        assert_eq!(packets[2].packet_id, PacketId::MessageBinary);
        assert_eq!(packets[2].data, Bytes::from_static(b"Hello"));

        assert_eq!(Bytes::try_from(packets).unwrap(), data);
    }

    #[test]
    fn test_packet_id_conversion_and_incompl_packet() -> Result<()> {
        let sut = Packet::try_from(Bytes::from_static(b"4"));
        assert!(sut.is_err());
        let _sut = sut.unwrap_err();
        assert!(matches!(Error::IncompletePacket, _sut));

        assert_eq!(PacketId::MessageBinary.to_string(), "b");

        let sut = PacketId::try_from(b'0')?;
        assert_eq!(sut, PacketId::Open);
        assert_eq!(sut.to_string(), "0");

        let sut = PacketId::try_from(b'1')?;
        assert_eq!(sut, PacketId::Close);
        assert_eq!(sut.to_string(), "1");

        let sut = PacketId::try_from(b'2')?;
        assert_eq!(sut, PacketId::Ping);
        assert_eq!(sut.to_string(), "2");

        let sut = PacketId::try_from(b'3')?;
        assert_eq!(sut, PacketId::Pong);
        assert_eq!(sut.to_string(), "3");

        let sut = PacketId::try_from(b'4')?;
        assert_eq!(sut, PacketId::Message);
        assert_eq!(sut.to_string(), "4");

        let sut = PacketId::try_from(b'5')?;
        assert_eq!(sut, PacketId::Upgrade);
        assert_eq!(sut.to_string(), "5");

        let sut = PacketId::try_from(b'6')?;
        assert_eq!(sut, PacketId::Noop);
        assert_eq!(sut.to_string(), "6");

        let sut = PacketId::try_from(42);
        assert!(sut.is_err());
        assert!(matches!(sut.unwrap_err(), Error::InvalidPacketId(42)));

        Ok(())
    }

    #[test]
    fn test_handshake_packet() {
        assert!(
            HandshakePacket::try_from(Packet::new(PacketId::Message, Bytes::from("test"))).is_err()
        );
        let packet = HandshakePacket {
            ping_interval: 10000,
            ping_timeout: 1000,
            sid: "Test".to_owned(),
            upgrades: vec!["websocket".to_owned(), "test".to_owned()],
        };
        let encoded: String = serde_json::to_string(&packet).unwrap();

        assert_eq!(
            packet,
            HandshakePacket::try_from(Packet::new(PacketId::Message, Bytes::from(encoded)))
                .unwrap()
        );
    }
}
