use crossbeam::sync::MsQueue;
use ether_dream::dac;
use ether_dream::protocol::{self, Command, SizeBytes, WriteBytes};
use std::{net, time};
use std::io::{self, Write};
use std::sync::Arc;
use stream::{self, Stream};

/// Listens for and handles requests to connect with the DAC.
pub struct Listener {
    // The state of the listener.
    //
    // This is always `Some` - Option is used to work around ownership issues in the `run` method.
    state: Option<State>,
    // The TCP listener used for handling connection requests.
    tcp_listener: net::TcpListener,
    // The rate at which each stream's output processor will emit "frames" of points.
    output_frame_rate: u32,
}

// Used to receive the state of the DAC once the **Stream** is finished with it.
type DacQueue = MsQueue<dac::Addressed>;

// The current state of the listener.
enum State {
    // The listener is currently waiting to accept the next connection request.
    Waiting(dac::Addressed),
    // The DAC is currently connected to a stream and is not currently accepting requests.
    //
    // The inner **DacQueue** is used to receive the state of the DAC once the **Stream** is
    // finished with it.
    Connected(Arc<DacQueue>),
}

/// A handle to an active **Stream** thread.
///
/// The **ActiveStream** can be used to produce a stream **Output**. The **stream::Output** yields
/// "frames" of points emitted by the inner **stream::output::Processor** thread.
pub struct ActiveStream {
    inner: Option<(stream::Handle, Arc<DacQueue>)>,
}

impl Listener {
    /// Create a new listener for handling stream connection requests.
    pub fn new(dac: dac::Addressed, output_frame_rate: u32) -> io::Result<Self> {
        let addr = net::SocketAddrV4::new([0, 0, 0, 0].into(), protocol::COMMUNICATION_PORT);
        let tcp_listener = net::TcpListener::bind(addr)?;
        let state = Some(State::Waiting(dac));
        Ok(Listener { state, tcp_listener, output_frame_rate })
    }

    /// Waits for an incoming TCP connection request and connects.
    ///
    /// The **Listener** will only accept one **Stream** at a time. If this method is called while
    /// an accepted stream still exists, this will block until that **Stream** is `close`d or
    /// `drop`ped.
    pub fn accept(&mut self) -> io::Result<(ActiveStream, net::SocketAddr)> {
        loop {
            match self.state.take().expect("listener state was `None`") {
                State::Waiting(dac) => {
                    // Accept the TCP stream.
                    let (mut tcp_stream, source_addr) = self.tcp_listener.accept()?;

                    // Set the timeout on the TCP stream to 1 second as per the protocol.
                    tcp_stream.set_read_timeout(Some(time::Duration::from_secs(1)))?;
                    tcp_stream.set_write_timeout(Some(time::Duration::from_secs(1)))?;

                    // When the connection to the DAC is first established, it responds sends a
                    // `DacResponse` as though responding to a `Ping`.
                    {
                        let response = protocol::DacResponse::ACK;
                        let command = protocol::command::Ping::START_BYTE;
                        let dac_status = dac.status.to_protocol();
                        let dac_response = protocol::DacResponse {
                            response,
                            command,
                            dac_status,
                        };
                        let mut bytes = [0u8; protocol::DacResponse::SIZE_BYTES];
                        (&mut bytes[..]).write_bytes(&dac_response)?;
                        tcp_stream.write(&bytes)?;
                    }

                    // Create and spawn the stream.
                    let stream = Stream::new(dac, tcp_stream, self.output_frame_rate)?.spawn()?;
                    // The queue for sending the DAC state back to the listener when shutdown.
                    let dac_queue = Arc::new(MsQueue::new());

                    // Update the **Listener** state.
                    let new_state = State::Connected(dac_queue.clone());
                    self.state = Some(new_state);

                    // Create and return the active stream.
                    let active_stream = ActiveStream { inner: Some((stream, dac_queue)) };
                    return Ok((active_stream, source_addr));
                },
                State::Connected(dac_rx) => {
                    let mut dac = dac_rx.pop();
                    // Reset DAC status.
                    dac.status = dac::Status::from_protocol(&super::initial_status()).unwrap();
                    self.state = Some(State::Waiting(dac));
                },
            }
        }
    }
}

impl ActiveStream {
    /// Produce a stream **Output**.
    ///
    /// The **stream::Output** yields "frames" of points emitted by the inner
    /// **stream::output::Processor** thread. These frames are intended to be useful for debugging
    /// or visualisation.
    pub fn output(&self) -> stream::Output {
        let (stream, _) = self.inner.as_ref()
            .expect("`output` was called but stream has been closed");
        stream.output()
    }

    /// Wait for the stream to be closed by the user and return the reason for shutdown.
    pub fn wait(mut self) -> io::Error {
        let (stream, dac_queue) = self.inner.take()
            .expect("`wait` was called but stream has already been closed");
        let (dac, io_err) = stream.wait();
        dac_queue.push(dac);
        io_err
    }

    // Shared between the **drop** and **close** methods.
    fn close_inner(&mut self) -> io::Error {
        let (stream, dac_queue) = self.inner.take()
            .expect("`close` was called but stream has already been closed");
        let (dac, io_err) = stream.close();
        dac_queue.push(dac);
        io_err
    }

    /// Close the TCP connection.
    ///
    /// This returns the inner DAC state to the **Listener** and allows the listener to accept new
    /// connections.
    pub fn close(mut self) -> io::Error {
        self.close_inner()
    }
}

impl Drop for ActiveStream {
    fn drop(&mut self) {
        self.close_inner();
    }
}
