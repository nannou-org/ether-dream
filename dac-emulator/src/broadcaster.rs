//! Items related to the DAC emulator's broadcasting process.

use ether_dream::dac;
use ether_dream::protocol::{DacBroadcast, SizeBytes, WriteBytes, BROADCAST_PORT};
use futures::prelude::*;
use smol::Async;
use std::{io, net, time};

/// The broadcasting side of the DAC.
///
/// Once `run` is called, the broadcaster will block and loop at a rate of once-per-second,
/// broadcasting the current state of the DAC on each iteration.
pub struct Broadcaster {
    // The last received copy of the state of the DAC.
    dac: dac::Addressed,
    // Used for broadcasting.
    udp_socket: Async<net::UdpSocket>,
    // The address to which the broadcaster is sending.
    broadcast_addr: net::SocketAddrV4,
    // A buffer to use for writing `DacBroadcast`s to bytes for the UDP socket.
    bytes: [u8; DacBroadcast::SIZE_BYTES],
}

/// **Broadcaster::run** blocks on the following messages.
#[derive(Debug)]
pub enum Message {
    /// The state of the DAC has changed.
    Dac(dac::Addressed),
    /// Tell the broadcaster to send a message.
    Send,
    /// Tell the broadcaster to stop running and break from its loop.
    Close,
}

impl Broadcaster {
    /// Create a new **Broadcaster** initialised with the given DAC state.
    ///
    /// Produces an **io::Error** if creating the UDP socket fails or if enabling broadcast fails.
    pub fn new(
        dac: dac::Addressed,
        bind_port: u16,
        broadcast_ip: net::Ipv4Addr,
    ) -> io::Result<Broadcaster> {
        let broadcast_addr = net::SocketAddrV4::new(broadcast_ip, BROADCAST_PORT);
        let bind_addr = net::SocketAddrV4::new([0, 0, 0, 0].into(), bind_port);
        let udp_socket = Async::<net::UdpSocket>::bind(bind_addr)?;
        udp_socket.get_ref().set_broadcast(true)?;
        let bytes = [0u8; DacBroadcast::SIZE_BYTES];
        Ok(Broadcaster {
            dac,
            udp_socket,
            broadcast_addr,
            bytes,
        })
    }

    /// Creates a **DacBroadcast** from the current known DAC state.
    ///
    /// This is used within the **send** method.
    pub fn create_broadcast(&self) -> DacBroadcast {
        // Retrieve the current DAC status.
        let dac_status = self.dac.status.to_protocol();
        // Create the broadcast message.
        DacBroadcast {
            mac_address: self.dac.mac_address.into(),
            hw_revision: self.dac.hw_revision,
            sw_revision: self.dac.sw_revision,
            buffer_capacity: self.dac.buffer_capacity,
            max_point_rate: self.dac.max_point_rate,
            dac_status,
        }
    }

    /// Sends a single broadcast message over the inner UDP socket.
    pub async fn send(&mut self) -> io::Result<()> {
        {
            // Write the broadcast to the bytes buffer.
            let dac_broadcast = self.create_broadcast();
            let mut writer = &mut self.bytes[..];
            writer.write_bytes(&dac_broadcast)?;
        }
        // Send the broadcast bytes over the UDP socket.
        let addr = self.broadcast_addr.clone();
        self.udp_socket.send_to(&self.bytes, addr).await?;
        Ok(())
    }

    /// Run the **Broadcaster** with the given stream of **Message**s.
    ///
    /// On each **Message::Send**, the **Broadcaster** will send a message over UDP.
    ///
    /// On each **Message::Dac**, the **Broadcaster** will update the current DAC state.
    ///
    /// On **Message::Close**, the stream will complete, in turn signalling the completion of the
    /// future returned by this method.
    pub async fn run<M>(&mut self, mut msgs: M) -> io::Result<()>
    where
        M: Stream<Item = Message> + Unpin,
    {
        // Loop forever handling each message.
        while let Some(msg) = msgs.next().await {
            match msg {
                // If a second has passed, send a new broadcast message.
                Message::Send => self.send().await?,
                // Update the state of the DAC.
                Message::Dac(dac) => self.dac = dac,
                // Break from the loop.
                Message::Close => break,
            }
        }
        Ok(())
    }
}

/// Creates a stream that yields at a rate specified by the given interval duration.
pub fn timer_stream(interval: time::Duration) -> impl Stream<Item = ()> {
    let delay_iter = (0..).map(move |_| smol::Timer::after(interval));
    let timer = futures::stream::iter(delay_iter);
    timer.filter_map(|delay| async move {
        delay.await;
        Some(())
    })
}

/// A stream that yields the next `Item` once per second.
pub fn one_hz() -> impl Stream<Item = ()> {
    timer_stream(time::Duration::from_secs(1))
}

/// A stream that emits a single `Message::Send` once per second.
pub fn one_hz_send() -> impl Stream<Item = Message> {
    one_hz().map(|()| Message::Send)
}
