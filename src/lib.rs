//! A full implementation of the Ether Dream laser protocol.

#[macro_use] extern crate bitflags;
extern crate byteorder;

pub mod dac;
pub mod protocol;

use protocol::{ReadBytes, SizeBytes};
use std::{io, net};

/// An iterator that listens and waits for broadcast messages from DACs on the network and yields
/// them as they are received on the inner UDP socket.
///
/// Yields an `io::Error` if:
///
/// - An error occurred when receiving on the inner UDP socket.
/// - A `DacBroadcast` could not be read from the received bytes.
pub struct RecvDacBroadcasts {
    udp_socket: net::UdpSocket,
    buffer: [u8; RecvDacBroadcasts::BUFFER_LEN],
}

impl RecvDacBroadcasts {
    /// The size of the inner buffer used to receive broadcast messages.
    pub const BUFFER_LEN: usize = protocol::DacBroadcast::SIZE_BYTES;
}

/// Produces an iterator that listens and waits for broadcast messages from DACs on the network and
/// yields them as they are received on the inner UDP socket.
///
/// This function returns an `io::Error` if it could not bind to the broadcast address
/// `255.255.255.255:<protocol::BROADCAST_PORT>`.
///
/// The produced iterator yields an `io::Error` if:
///
/// - An error occurred when receiving on the inner UDP socket.
/// - A `DacBroadcast` could not be read from the received bytes.
///
/// ## Example
///
/// ```no_run
/// extern crate ether_dream;
/// 
/// fn main() {
///     let dac_broadcasts = ether_dream::recv_dac_broadcasts().expect("failed to bind to UDP socket");
///     for dac_broadcast in dac_broadcasts {
///         println!("{:#?}", dac_broadcast);
///     }
/// }
/// ```
pub fn recv_dac_broadcasts() -> io::Result<RecvDacBroadcasts> {
    let broadcast_port = protocol::BROADCAST_PORT;
    let broadcast_addr = net::SocketAddrV4::new([255, 255, 255, 255].into(), broadcast_port);
    let udp_socket = net::UdpSocket::bind(broadcast_addr)?;
    let buffer = [0; RecvDacBroadcasts::BUFFER_LEN];
    Ok(RecvDacBroadcasts { udp_socket, buffer })
}

impl Iterator for RecvDacBroadcasts {
    type Item = io::Result<(protocol::DacBroadcast, net::SocketAddr)>;
    fn next(&mut self) -> Option<Self::Item> {
        let RecvDacBroadcasts {
            ref mut buffer,
            ref mut udp_socket,
        } = *self;
        let (_len, src_addr) = match udp_socket.recv_from(buffer) {
            Ok(msg) => msg,
            Err(err) => return Some(Err(err)),
        };
        let mut bytes = &buffer[..];
        let dac_broadcast = match bytes.read_bytes::<protocol::DacBroadcast>() {
            Ok(dac_broadcast) => dac_broadcast,
            Err(err) => return Some(Err(err)),
        };
        Some(Ok((dac_broadcast, src_addr)))
    }
}
