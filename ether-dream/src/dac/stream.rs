use crate::dac;
use crate::protocol::{self, Command, ReadBytes, SizeBytes, WriteBytes, WriteToBytes};
use std::borrow::Cow;
use std::error::Error;
use std::io::{self, Read, Write};
use std::{self, fmt, mem, net, ops, time};

/// A bi-directional communication stream between the user and a `Dac`.
pub struct Stream {
    /// An up-to-date representation of the `DAC` with which the stream is connected.
    dac: dac::Addressed,
    /// The TCP stream used for communicating with the DAC.
    tcp_stream: net::TcpStream,
    /// A buffer to re-use for queueing commands via the `queue_commands` method.
    command_buffer: Vec<QueuedCommand>,
    /// A buffer to re-use for queueing points for `Data` commands.
    point_buffer: Vec<protocol::DacPoint>,
    /// A buffer used for efficiently writing and reading bytes to and from TCP.
    bytes: Vec<u8>,
}

/// A runtime representation of any of the possible commands.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum QueuedCommand {
    PrepareStream,
    Begin(protocol::command::Begin),
    PointRate(protocol::command::PointRate),
    Data(ops::Range<usize>),
    Stop,
    EmergencyStop,
    ClearEmergencyStop,
    Ping,
}

/// A queue of commands that are to be submitted at once before listening for their responses.
pub struct CommandQueue<'a> {
    stream: &'a mut Stream,
}

/// Errors that may occur when connecting a `Stream`.
#[derive(Debug)]
pub enum CommunicationError {
    Io(io::Error),
    Protocol(dac::ProtocolError),
    Response(ResponseError),
}

/// An error indicated by a DAC response.
#[derive(Debug)]
pub struct ResponseError {
    /// The DAC response on which the NAK was received.
    pub response: protocol::DacResponse,
    /// The kind of response error that occurred.
    pub kind: ResponseErrorKind,
}

/// The kinds of errors that may be interpreted from a DAC response.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ResponseErrorKind {
    /// The response was to a command that was unexpected.
    UnexpectedCommand(u8),
    /// The DAC responded with a NAK.
    Nak(Nak),
}

/// The NAK message kinds that may be returned by the DAC.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Nak {
    /// The write command could not be performed because there was not enough buffer space when it
    /// was received.
    Full,
    /// The command contained an invalid `command` byte or parameters.
    Invalid,
    /// An emergency-stop condition still exists.
    StopCondition,
}

impl Stream {
    fn send_command<C>(&mut self, command: C) -> io::Result<()>
    where
        C: Command + WriteToBytes,
    {
        let Stream {
            ref mut bytes,
            ref mut tcp_stream,
            ..
        } = *self;
        send_command(bytes, tcp_stream, command)
    }

    fn recv_response(&mut self, expected_command: u8) -> Result<(), CommunicationError> {
        let Stream {
            ref mut bytes,
            ref mut tcp_stream,
            ref mut dac,
            ..
        } = *self;
        recv_response(bytes, tcp_stream, dac, expected_command)
    }

    /// Borrow the inner DAC to examine its state.
    pub fn dac(&self) -> &dac::Addressed {
        &self.dac
    }

    /// Queue one or more commands to be submitted to the DAC at once.
    pub fn queue_commands(&mut self) -> CommandQueue {
        self.command_buffer.clear();
        self.point_buffer.clear();
        CommandQueue { stream: self }
    }

    /// This directly calls `set_nodelay` on the inner **TcpStream**. In other words, this sets the
    /// value of the TCP_NODELAY option for this socket.
    ///
    /// Note that due to the necessity for very low-latency communication with the DAC, this API
    /// enables TCP_NODELAY by default. This method is exposed in order to allow the user to
    /// disable this if they wish.
    ///
    /// When not set, data is buffered until there is a sufficient amount to send out, thereby
    /// avoiding the frequent sending of small packets. Although perhaps more efficient for the
    /// network, this may result in DAC underflows if **Data** commands are delayed for too long.
    pub fn set_nodelay(&self, b: bool) -> io::Result<()> {
        self.tcp_stream.set_nodelay(b)
    }

    /// Gets the value of the TCP_NODELAY option for this socket.
    ///
    /// For more infnormation about this option, see `set_nodelay`.
    pub fn nodelay(&self) -> io::Result<bool> {
        self.tcp_stream.nodelay()
    }

    /// This directly calls `set_ttl` on the inner **TcpStream**. In other words, this sets the
    /// value for the `IP_TTL` option on this socket.
    ///
    /// This value sets the time-to-live field that is used in every packet sent from this socket.
    /// Time-to-live describes the number of hops between devices that a packet may make before it
    /// is discarded/ignored.
    pub fn set_ttl(&self, ttl: u32) -> io::Result<()> {
        self.tcp_stream.set_ttl(ttl)
    }

    /// Gets the value of the `IP_TTL` option for this socket.
    ///
    /// For more information about this option see `set_ttl`.
    pub fn ttl(&self) -> io::Result<u32> {
        self.tcp_stream.ttl()
    }

    /// Sets the read timeout of the underlying `TcpStream`.
    ///
    /// See the following for details:
    ///
    /// - [`std::net::TcpStream::set_read_timeout`](https://doc.rust-lang.org/std/net/struct.TcpStream.html#method.set_read_timeout)
    pub fn set_read_timeout(&self, duration: Option<time::Duration>) -> io::Result<()> {
        self.tcp_stream.set_read_timeout(duration)
    }

    /// Sets the write timeout of the underlying `TcpStream`.
    ///
    /// See the following for details:
    ///
    /// - [`std::net::TcpStream::set_write_timeout`](https://doc.rust-lang.org/std/net/struct.TcpStream.html#method.set_write_timeout)
    pub fn set_write_timeout(&self, duration: Option<time::Duration>) -> io::Result<()> {
        self.tcp_stream.set_write_timeout(duration)
    }

    /// Sets the read and write timeout of the underlying `TcpStream`.
    ///
    /// See the following for details:
    ///
    /// - [`std::net::TcpStream::set_read_timeout`](https://doc.rust-lang.org/std/net/struct.TcpStream.html#method.set_read_timeout)
    /// - [`std::net::TcpStream::set_write_timeout`](https://doc.rust-lang.org/std/net/struct.TcpStream.html#method.set_write_timeout)
    pub fn set_timeout(&self, duration: Option<time::Duration>) -> io::Result<()> {
        self.set_read_timeout(duration)?;
        self.set_write_timeout(duration)
    }
}

impl<'a> CommandQueue<'a> {
    /// This command causes the playback system to enter the `Prepared` state. The DAC resets its
    /// buffer to be empty and sets "point_count" to `0`.
    ///
    /// This command may only be sent if the light engine is `Ready` and the playback system is
    /// `Idle`. If so, the DAC replies with ACK. Otherwise, it replies with NAK - Invalid.
    pub fn prepare_stream(self) -> Self {
        self.stream
            .command_buffer
            .push(QueuedCommand::PrepareStream);
        self
    }

    /// Causes the DAC to begin producing output.
    ///
    /// ### `low_water_mark`
    ///
    /// *Currently unused.*
    ///
    /// ### `point_rate`
    ///
    /// The number of points per second to be read from the buffer.  If the playback system was
    /// `Prepared` and there was data in the buffer, then the DAC will reply with ACK. Otherwise,
    /// it replies with NAK - Invalid.
    pub fn begin(self, low_water_mark: u16, point_rate: u32) -> Self {
        let begin = protocol::command::Begin {
            low_water_mark,
            point_rate,
        };
        self.stream.command_buffer.push(QueuedCommand::Begin(begin));
        self
    }

    /// Adds a new point rate to the point rate buffer.
    ///
    /// Point rate changes are read out of the buffer when a point with an appropriate flag is
    /// played (see the `WriteData` command).
    ///
    /// If the DAC's playback state is not `Prepared` or `Playing`, it replies with NAK - Invalid.
    ///
    /// If the point rate buffer is full, it replies with NAK - Full.
    ///
    /// Otherwise, it replies with ACK.
    pub fn point_rate(self, point_rate: u32) -> Self {
        let point_rate = protocol::command::PointRate(point_rate);
        self.stream
            .command_buffer
            .push(QueuedCommand::PointRate(point_rate));
        self
    }

    /// Indicates to the DAC to add the following point data into its buffer.
    pub fn data<I>(self, points: I) -> Self
    where
        I: IntoIterator<Item = protocol::DacPoint>,
    {
        let start = self.stream.point_buffer.len();
        self.stream.point_buffer.extend(points);
        let end = self.stream.point_buffer.len();
        assert!(
            end - start < std::u16::MAX as usize,
            "the number of points exceeds the `u16` MAX"
        );
        self.stream
            .command_buffer
            .push(QueuedCommand::Data(start..end));
        self
    }

    /// Causes the DAC to immediately stop playing and return to the `Idle` playback state.
    ///
    /// It is ACKed if the DAC was in the `Playing` or `Prepared` playback states.
    ///
    /// Otherwise it is replied to with NAK - Invalid.
    pub fn stop(self) -> Self {
        self.stream.command_buffer.push(QueuedCommand::Stop);
        self
    }

    /// Causes the light engine to enter the E-Stop state, regardless of its previous state.
    ///
    /// This command is always ACKed.
    pub fn emergency_stop(self) -> Self {
        self.stream.command_buffer.push(QueuedCommand::Stop);
        self
    }

    /// If the light engine was in E-Stop state due to an emergency stop command (either from a
    /// local stop condition or over the network), this command resets it to `Ready`.
    ///
    /// It is ACKed if the DAC was previously in E-Stop.
    ///
    /// Otherwise it is replied to with a NAK - Invalid.
    ///
    /// If the condition that caused the emergency stop is still active (e.g. E-Stop input still
    /// asserted, temperature still out of bounds, etc) a NAK - Stop Condition is sent.
    pub fn clear_emergency_stop(self) -> Self {
        self.stream.command_buffer.push(QueuedCommand::Stop);
        self
    }

    /// The DAC will reply to this with an ACK packet.
    ///
    /// This serves as a keep-alive for the connection when the DAC is not actively streaming.
    pub fn ping(self) -> Self {
        self.stream.command_buffer.push(QueuedCommand::Ping);
        self
    }

    /// Finish queueing commands and send them to the DAC.
    ///
    /// First all commands are written to the TCP stream, then we block waiting for a response to
    /// each from the DAC.
    pub fn submit(self) -> Result<(), CommunicationError> {
        let CommandQueue { stream } = self;

        // Track the command byte order so that we can ensure we get correct responses.
        let mut command_bytes = vec![];

        // Retrieve the command buffer so we can drain it.
        let mut command_buffer = mem::replace(&mut stream.command_buffer, Vec::new());

        // Send each command via the TCP stream.
        for command in command_buffer.drain(..) {
            match command {
                QueuedCommand::PrepareStream => {
                    stream.send_command(protocol::command::PrepareStream)?;
                    command_bytes.push(protocol::command::PrepareStream::START_BYTE);
                }
                QueuedCommand::Begin(begin) => {
                    stream.send_command(begin)?;
                    command_bytes.push(protocol::command::Begin::START_BYTE);
                }
                QueuedCommand::PointRate(point_rate) => {
                    stream.send_command(point_rate)?;
                    command_bytes.push(protocol::command::PointRate::START_BYTE);
                }
                QueuedCommand::Data(range) => {
                    let Stream {
                        ref mut bytes,
                        ref mut tcp_stream,
                        ref point_buffer,
                        ..
                    } = *stream;
                    let points = Cow::Borrowed(&point_buffer[range]);
                    let data = protocol::command::Data { points };
                    send_command(bytes, tcp_stream, data)?;
                    command_bytes.push(protocol::command::Data::START_BYTE);
                }
                QueuedCommand::Stop => {
                    stream.send_command(protocol::command::Stop)?;
                    command_bytes.push(protocol::command::Stop::START_BYTE);
                }
                QueuedCommand::EmergencyStop => {
                    stream.send_command(protocol::command::EmergencyStop)?;
                    command_bytes.push(protocol::command::EmergencyStop::START_BYTE);
                }
                QueuedCommand::ClearEmergencyStop => {
                    stream.send_command(protocol::command::ClearEmergencyStop)?;
                    command_bytes.push(protocol::command::ClearEmergencyStop::START_BYTE);
                }
                QueuedCommand::Ping => {
                    stream.send_command(protocol::command::Ping)?;
                    command_bytes.push(protocol::command::Ping::START_BYTE);
                }
            }
        }

        // Place the allocated command buffer back in the stream.
        mem::swap(&mut stream.command_buffer, &mut command_buffer);

        // Wait for a response to each command.
        for command_byte in command_bytes.drain(..) {
            stream.recv_response(command_byte)?;
        }

        Ok(())
    }
}

impl protocol::DacResponse {
    // Checks the response for unexpected command and NAK errors.
    fn check_errors(&self, expected_command: u8) -> Result<(), ResponseError> {
        if self.command != expected_command {
            let response = self.clone();
            let kind = ResponseErrorKind::UnexpectedCommand(self.command);
            let err = ResponseError { response, kind };
            return Err(err);
        }

        if let Some(nak) = Nak::from_protocol(self.response) {
            let response = self.clone();
            let kind = ResponseErrorKind::Nak(nak);
            let err = ResponseError { response, kind };
            return Err(err);
        }

        Ok(())
    }
}

impl Nak {
    /// Produce a `Nak` from the low-level protocol byte representation.
    pub fn from_protocol(nak: u8) -> Option<Self> {
        match nak {
            protocol::DacResponse::NAK_FULL => Some(Nak::Full),
            protocol::DacResponse::NAK_INVALID => Some(Nak::Invalid),
            protocol::DacResponse::NAK_STOP_CONDITION => Some(Nak::StopCondition),
            _ => None,
        }
    }

    /// Convert the `Nak` to the low-level protocol byte representation.
    pub fn to_protocol(&self) -> u8 {
        match *self {
            Nak::Full => protocol::DacResponse::NAK_FULL,
            Nak::Invalid => protocol::DacResponse::NAK_INVALID,
            Nak::StopCondition => protocol::DacResponse::NAK_STOP_CONDITION,
        }
    }
}

impl Error for CommunicationError {
    fn cause(&self) -> Option<&dyn Error> {
        match *self {
            CommunicationError::Io(ref err) => Some(err as _),
            CommunicationError::Protocol(ref err) => Some(err as _),
            CommunicationError::Response(ref err) => Some(err as _),
        }
    }
}

impl Error for ResponseError {}

impl fmt::Display for CommunicationError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            CommunicationError::Io(ref err) => err.fmt(f),
            CommunicationError::Protocol(ref err) => err.fmt(f),
            CommunicationError::Response(ref err) => err.fmt(f),
        }
    }
}

impl fmt::Display for ResponseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let s = match self.kind {
            ResponseErrorKind::UnexpectedCommand(_) => {
                "the received response was to an unexpected command"
            }
            ResponseErrorKind::Nak(ref nak) => match *nak {
                Nak::Full => "DAC responded with \"NAK - Full\"",
                Nak::Invalid => "DAC responded with \"NAK - Invalid\"",
                Nak::StopCondition => "DAC responded with \"NAK - Stop Condition\"",
            },
        };
        write!(f, "{}", s)
    }
}

impl From<io::Error> for CommunicationError {
    fn from(err: io::Error) -> Self {
        CommunicationError::Io(err)
    }
}

impl From<dac::ProtocolError> for CommunicationError {
    fn from(err: dac::ProtocolError) -> Self {
        CommunicationError::Protocol(err)
    }
}

impl From<ResponseError> for CommunicationError {
    fn from(err: ResponseError) -> Self {
        CommunicationError::Response(err)
    }
}

/// Establishes a TCP stream connection with the DAC at the given address.
///
/// `TCP_NODELAY` is enabled on the TCP stream in order for better low-latency/realtime
/// suitability. If necessary, this can be disabled via the `set_nodelay` method on the returned
/// **Stream**.
///
/// Note that this does not "prepare" the DAC for playback. This must be done manually by
/// submitting the `prepare_stream` command.
pub fn connect(
    broadcast: &protocol::DacBroadcast,
    dac_ip: net::IpAddr,
) -> Result<Stream, CommunicationError> {
    connect_inner(broadcast, dac_ip, &net::TcpStream::connect)
}

/// Establishes a TCP stream connection with the DAC at the given address.
///
/// This behaves the same as `connect`, but times out after the given duration.
pub fn connect_timeout(
    broadcast: &protocol::DacBroadcast,
    dac_ip: net::IpAddr,
    timeout: time::Duration,
) -> Result<Stream, CommunicationError> {
    let connect = |addr| net::TcpStream::connect_timeout(&addr, timeout);
    connect_inner(broadcast, dac_ip, &connect)
}

/// Shared between the `connect` and `connect_timeout` implementations.
fn connect_inner(
    broadcast: &protocol::DacBroadcast,
    dac_ip: net::IpAddr,
    connect: &dyn Fn(net::SocketAddr) -> io::Result<net::TcpStream>,
) -> Result<Stream, CommunicationError> {
    // Initialise the DAC state representation.
    let mut dac = dac::Addressed::from_broadcast(broadcast)?;

    // Connect the TCP stream.
    let dac_addr = net::SocketAddr::new(dac_ip, protocol::COMMUNICATION_PORT);
    let mut tcp_stream = connect(dac_addr)?;

    // Enable `TCP_NODELAY` for better low-latency suitability.
    tcp_stream.set_nodelay(true)?;

    // Initialise a buffer for writing bytes to the TCP stream.
    let mut bytes = vec![];

    // Upon connection, the DAC responds as though it were sent a **Ping** command.
    recv_response(
        &mut bytes,
        &mut tcp_stream,
        &mut dac,
        protocol::command::Ping::START_BYTE,
    )?;

    // Create the stream.
    let stream = Stream {
        dac,
        tcp_stream,
        command_buffer: vec![],
        point_buffer: vec![],
        bytes,
    };

    Ok(stream)
}

/// Used within the `Stream::send_command` method.
fn send_command<C>(
    bytes: &mut Vec<u8>,
    tcp_stream: &mut net::TcpStream,
    command: C,
) -> io::Result<()>
where
    C: Command + WriteToBytes,
{
    bytes.clear();
    bytes.write_bytes(command)?;
    tcp_stream.write(bytes)?;
    Ok(())
}

/// Used within the `Stream::recv_response` and `Stream::connect` methods.
fn recv_response(
    bytes: &mut Vec<u8>,
    tcp_stream: &mut net::TcpStream,
    dac: &mut dac::Addressed,
    expected_command: u8,
) -> Result<(), CommunicationError> {
    // Read the response.
    bytes.resize(protocol::DacResponse::SIZE_BYTES, 0);
    tcp_stream.read_exact(bytes)?;
    let response = (&bytes[..]).read_bytes::<protocol::DacResponse>()?;
    response.check_errors(expected_command)?;
    // Update the DAC representation.
    dac.update_status(&response.dac_status)?;
    Ok(())
}
