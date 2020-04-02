//! A full implementation of the Ether Dream laser protocol.

#[macro_use]
extern crate bitflags;
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

/// Produces a `RecvDacBroadcasts` instance that listens and waits for broadcast messages from DACs
/// on the network and yields them as they are received on the inner UDP socket.
///
/// This function returns an `io::Error` if it could not bind to the broadcast address
/// `255.255.255.255:<protocol::BROADCAST_PORT>`.
///
/// The produced iterator yields an `io::Error` if:
///
/// - An error occurred when receiving on the inner UDP socket. This may include `TimedOut` or
/// `WouldBlock` if either of the `set_timeout` or `set_nonblocking` methods have been called.
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
    let broadcast_addr = net::SocketAddrV4::new([0, 0, 0, 0].into(), broadcast_port);
    let udp_socket = net::UdpSocket::bind(broadcast_addr)?;
    let buffer = [0; RecvDacBroadcasts::BUFFER_LEN];
    Ok(RecvDacBroadcasts { udp_socket, buffer })
}

impl RecvDacBroadcasts {
    /// Attempt to read the next broadcast.
    ///
    /// This method may or may not block depending on whether `set_nonblocking` has been called. By
    /// default, this method will block.
    pub fn next_broadcast(&mut self) -> io::Result<(protocol::DacBroadcast, net::SocketAddr)> {
        let RecvDacBroadcasts {
            ref mut buffer,
            ref mut udp_socket,
        } = *self;
        let (_len, src_addr) = udp_socket.recv_from(buffer)?;
        let mut bytes = &buffer[..];
        let dac_broadcast = bytes.read_bytes::<protocol::DacBroadcast>()?;
        Ok((dac_broadcast, src_addr))
    }

    /// Set the timeout for the inner UDP socket used for reading broadcasts.
    ///
    /// See the [`std::net::UdpSocket::set_read_timeout`
    /// docs](https://doc.rust-lang.org/std/net/struct.UdpSocket.html#method.set_read_timeout) for
    /// details.
    pub fn set_timeout(&self, duration: Option<std::time::Duration>) -> io::Result<()> {
        self.udp_socket.set_read_timeout(duration)
    }

    /// Moves the inner UDP socket into or out of nonblocking mode.
    ///
    /// See the
    /// [`std::net::UdpSocket::set_nonblocking`](https://doc.rust-lang.org/std/net/struct.UdpSocket.html#method.set_nonblocking)
    /// for details.
    pub fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
        self.udp_socket.set_nonblocking(nonblocking)
    }
}

impl Iterator for RecvDacBroadcasts {
    type Item = io::Result<(protocol::DacBroadcast, net::SocketAddr)>;
    fn next(&mut self) -> Option<Self::Item> {
        Some(self.next_broadcast())
    }
}
