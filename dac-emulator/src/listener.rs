use crate::stream::Stream;
use ether_dream::dac;
use ether_dream::protocol::{self, Command, SizeBytes, WriteBytes};
use futures::io::AsyncWriteExt;
use smol::Async;
use std::io;
use std::{net, time};

/// Listens for and handles requests to connect with the DAC.
pub struct Listener {
    // Information about the DAC we are listening for.
    dac: dac::Addressed,
    // The TCP listener used for handling connection requests.
    tcp_listener: Async<net::TcpListener>,
    // The most recently yieded TCP stream.
    stream: Option<Stream>,
}

impl Listener {
    /// Create a new listener for handling stream connection requests.
    pub fn new(dac: dac::Addressed) -> io::Result<Self> {
        let addr = net::SocketAddrV4::new([0, 0, 0, 0].into(), protocol::COMMUNICATION_PORT);
        let tcp_listener = Async::<net::TcpListener>::bind(addr)?;
        let stream = None;
        Ok(Listener {
            dac,
            tcp_listener,
            stream,
        })
    }

    /// Returns a future that waits for an incoming TCP connection request and connects.
    ///
    /// The **Listener** will only accept one **Stream** at a time. If this method is called while
    /// an accepted stream still exists, the resulting future will wait until the previously
    /// yielded **Stream** is `close`d or `drop`ped.
    pub async fn accept(&mut self) -> io::Result<(&mut Stream, net::SocketAddr)> {
        // Accept the TCP stream.
        let (mut tcp_stream, source_addr) = self.tcp_listener.accept().await?;

        // Set the timeout on the TCP stream to 1 second as per the protocol.
        tcp_stream
            .get_ref()
            .set_read_timeout(Some(time::Duration::from_secs(1)))?;
        tcp_stream
            .get_ref()
            .set_write_timeout(Some(time::Duration::from_secs(1)))?;
        // Enable `TCP_NODELAY`.
        tcp_stream.get_ref().set_nodelay(true)?;

        // When the connection to the DAC is first established, it responds sends a
        // `DacResponse` as though responding to a `Ping`.
        {
            let response = protocol::DacResponse::ACK;
            let command = protocol::command::Ping::START_BYTE;
            let dac_status = self.dac.status.to_protocol();
            let dac_response = protocol::DacResponse {
                response,
                command,
                dac_status,
            };
            let mut bytes = [0u8; protocol::DacResponse::SIZE_BYTES];
            (&mut bytes[..]).write_bytes(&dac_response)?;
            tcp_stream.write(&bytes).await?;
        }

        // Create and spawn the stream.
        let stream = Stream::new(self.dac, tcp_stream)?;
        self.stream = Some(stream);
        return Ok((self.stream.as_mut().unwrap(), source_addr));
    }
}
