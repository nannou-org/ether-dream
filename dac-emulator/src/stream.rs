//! The TCP stream between a client and the DAC.

use ether_dream::dac;
use ether_dream::protocol::{
    self, command, Command as CommandTrait, ReadBytes, SizeBytes, WriteBytes,
};
use futures::future::FutureExt;
use futures::io::{AsyncReadExt, AsyncWriteExt};
use piper::Mutex;
use smol::Async;
use std::collections::VecDeque;
use std::{io, net};

/// A stream of communication between a DAC and a user.
///
/// The stream expects to receive **Command**s and responseds with **DacResponse**s.
///
/// All communication occurs over a single TCP stream.
pub struct Stream {
    tcp: Tcp,
    shared: Mutex<Shared>,
}

struct Shared {
    dac: dac::Addressed,
    state: State,
}

struct Tcp {
    stream: Async<net::TcpStream>,
    bytes: Box<[u8]>,
}

#[derive(Default)]
pub struct State {
    // The moment at which the next point should be emitted.
    next_point_emission: Option<std::time::Instant>,
    // The DAC's buffer of points. The length should never exceed `buffer_capacity`.
    buffer: VecDeque<protocol::DacPoint>,
}

/// The stream DAC output stream underflowed due to insufficient data.
#[derive(Debug)]
pub struct Underflowed;

/// Commands that the DAC may be receive via a **Stream**.
#[derive(Debug)]
pub enum Command {
    PrepareStream(command::PrepareStream),
    Begin(command::Begin),
    PointRate(command::PointRate),
    Data(command::Data<'static>),
    Stop(command::Stop),
    EmergencyStop(command::EmergencyStop),
    ClearEmergencyStop(command::ClearEmergencyStop),
    Ping(command::Ping),
}

/// An attempt at interpreting a **Command** from bytes.
#[derive(Debug)]
pub enum InterpretedCommand {
    /// A successfuly interpreted, known command.
    Known { command: Command },
    /// Received an unknown command that started with the given byte.
    Unknown { start_byte: u8 },
}

impl Stream {
    /// Initialise a new **Stream**.
    ///
    /// Internally this allocates a buffer of bytes whose size is the size of the largest possible
    /// **Data** command that may be received based on the DAC's buffer capacity.
    ///
    /// Assumes `TCP_NODELAY` is already enabled on the given TCP socket in order to adhere to the
    /// low-latency, realtime requirements.
    ///
    /// This function also spawns a thread used for processing output.
    pub fn new(dac: dac::Addressed, stream: Async<net::TcpStream>) -> io::Result<Self> {
        // Prepare a buffer with the maximum expected command size.
        let bytes: Box<[u8]> = {
            let data_command_size_bytes = 1;
            let data_len_size_bytes = 2;
            let max_points_size_bytes =
                dac.buffer_capacity as usize * protocol::DacPoint::SIZE_BYTES;
            let max_command_size =
                data_command_size_bytes + data_len_size_bytes + max_points_size_bytes;
            vec![0u8; max_command_size].into()
        };
        let state = State::default();
        let shared = Mutex::new(Shared { dac, state });
        let tcp = Tcp { stream, bytes };
        Ok(Stream { tcp, shared })
    }

    /// Produce the current state of the DAC.
    pub fn dac(&self) -> dac::Addressed {
        self.shared.lock().dac.clone()
    }

    /// Handle TCP messages received on the given stream by attempting to interpret them as commands.
    ///
    /// Once processed, each command will be responded to.
    ///
    /// This runs forever until either an error occurs or the TCP stream is shutdown.
    ///
    /// Returns the **io::Error** that caused the loop to end.
    // TODO: Should emit `Event`s, where an event can be points or a tcp communication event.
    pub async fn next_points(
        &mut self,
    ) -> io::Result<Result<Vec<protocol::DacPoint>, Underflowed>> {
        let Stream {
            ref shared,
            ref mut tcp,
        } = *self;
        let err = loop {
            futures::select! {
                res = emit_points(shared).fuse() => return Ok(res),
                res = read_command_via_tcp_and_respond(shared, tcp).fuse() => match res {
                    Ok(()) => continue,
                    Err(err) => break err,
                },
            };
        };

        // Ensure the TCP stream is shutdown before exiting the thread.
        self.tcp.stream.get_ref().shutdown(net::Shutdown::Both).ok();

        // TODO: Currently can only return err coz no safe way to close stream (e.g. via msg).
        Err(err)
    }
}

impl InterpretedCommand {
    /// Read a single command from the TCP stream and return it.
    ///
    /// This method blocks until the exact number of bytes necessary for the returned command are
    /// read.
    pub async fn read_from_tcp_stream(
        bytes: &mut [u8],
        tcp_stream: &mut Async<net::TcpStream>,
    ) -> io::Result<Self> {
        // Peek the first byte to determine the command kind.
        bytes[0] = 0u8;
        let len = tcp_stream.peek(&mut bytes[..1]).await?;

        // Empty messages should only happen if the stream has closed.
        if len == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "read `0` bytes from tcp stream",
            ));
        }

        // Read the rest of the command from the stream based on the starting byte.
        let interpreted_command = match bytes[0] {
            command::PrepareStream::START_BYTE => {
                tcp_stream
                    .read_exact(&mut bytes[..command::PrepareStream::SIZE_BYTES])
                    .await?;
                let prepare_stream = (&bytes[..]).read_bytes::<command::PrepareStream>()?;
                Command::PrepareStream(prepare_stream).into()
            }
            command::Begin::START_BYTE => {
                tcp_stream
                    .read_exact(&mut bytes[..command::Begin::SIZE_BYTES])
                    .await?;
                let begin = (&bytes[..]).read_bytes::<command::Begin>()?;
                Command::Begin(begin).into()
            }
            command::PointRate::START_BYTE => {
                tcp_stream
                    .read_exact(&mut bytes[..command::PointRate::SIZE_BYTES])
                    .await?;
                let point_rate = (&bytes[..]).read_bytes::<command::PointRate>()?;
                Command::PointRate(point_rate).into()
            }
            command::Data::START_BYTE => {
                // Read the number of points.
                let command_bytes = 1;
                let n_points_bytes = 2;
                let n_points_start = command_bytes;
                let n_points_end = n_points_start + n_points_bytes;
                tcp_stream.peek(&mut bytes[..n_points_end]).await?;
                let n_points = command::Data::read_n_points(&bytes[n_points_start..n_points_end])?;

                // Use the number of points to determine how many bytes to read.
                let data_bytes = n_points as usize * protocol::DacPoint::SIZE_BYTES;
                let total_bytes = command_bytes + n_points_bytes + data_bytes;
                tcp_stream.read_exact(&mut bytes[..total_bytes]).await?;
                let data = (&bytes[..]).read_bytes::<command::Data<'static>>()?;
                Command::Data(data).into()
            }
            command::Stop::START_BYTE => {
                tcp_stream
                    .read_exact(&mut bytes[..command::Stop::SIZE_BYTES])
                    .await?;
                let stop = (&bytes[..]).read_bytes::<command::Stop>()?;
                Command::Stop(stop).into()
            }
            command::EmergencyStop::START_BYTE => {
                tcp_stream
                    .read_exact(&mut bytes[..command::EmergencyStop::SIZE_BYTES])
                    .await?;
                let emergency_stop = (&bytes[..]).read_bytes::<command::EmergencyStop>()?;
                Command::EmergencyStop(emergency_stop).into()
            }
            command::ClearEmergencyStop::START_BYTE => {
                tcp_stream
                    .read_exact(&mut bytes[..command::ClearEmergencyStop::SIZE_BYTES])
                    .await?;
                let clear_emergency_stop =
                    (&bytes[..]).read_bytes::<command::ClearEmergencyStop>()?;
                Command::ClearEmergencyStop(clear_emergency_stop).into()
            }
            command::Ping::START_BYTE => {
                tcp_stream
                    .read_exact(&mut bytes[..command::Ping::SIZE_BYTES])
                    .await?;
                let ping = (&bytes[..]).read_bytes::<command::Ping>()?;
                Command::Ping(ping).into()
            }
            start_byte => InterpretedCommand::Unknown { start_byte },
        };

        Ok(interpreted_command)
    }
}

impl From<Command> for InterpretedCommand {
    fn from(command: Command) -> Self {
        InterpretedCommand::Known { command }
    }
}

/// Produce a future representing the emission of the next point.
async fn emit_points(shared: &Mutex<Shared>) -> Result<Vec<protocol::DacPoint>, Underflowed> {
    let playback = shared.lock().dac.status.playback;
    if let dac::Playback::Playing = playback {
        let next = *shared
            .lock()
            .state
            .next_point_emission
            .get_or_insert_with(std::time::Instant::now);
        let timer = smol::Timer::at(next);
        timer.await;
        let mut guard = shared.lock();
        let shared = &mut *guard;
        let interval = rate_interval(shared.dac.status.point_rate);
        let next = shared.state.next_point_emission.as_mut().unwrap();
        let mut output = vec![];
        let now = std::time::Instant::now();
        while *next < now {
            *next += interval;
            if let Some(pt) = shared.state.buffer.pop_front() {
                output.push(pt);
            }
        }
        shared.dac.status.buffer_fullness = shared.state.buffer.len() as u16;
        shared.dac.status.point_count += output.len() as u32;
        let result = if output.is_empty() {
            Err(Underflowed)
        } else {
            Ok(output)
        };
        result
    } else {
        futures::future::pending::<_>().await
    }
}

/// Read a single command from the TCP stream and respond.
///
/// If a `read` occurred of `0` bytes, this function returns early and no response is sent.
async fn read_command_via_tcp_and_respond(shared: &Mutex<Shared>, tcp: &mut Tcp) -> io::Result<()> {
    // Read the command from the TCP stream.
    let interpreted_command =
        InterpretedCommand::read_from_tcp_stream(&mut tcp.bytes, &mut tcp.stream).await?;

    // Process command here.
    let dac_response = {
        let mut guard = shared.lock();
        let shared = &mut *guard;
        let dac = &mut shared.dac;
        let state = &mut shared.state;
        process_interpreted_command(dac, state, interpreted_command)
    };

    // Write the response to bytes.
    let response_len = protocol::DacResponse::SIZE_BYTES;
    (&mut tcp.bytes[..response_len]).write_bytes(&dac_response)?;
    tcp.stream.write(&tcp.bytes[..response_len]).await?;

    Ok(())
}

/// Process the interpreted command, update the DAC state accordingly and produce a response.
pub fn process_interpreted_command(
    dac: &mut dac::Addressed,
    state: &mut State,
    interpreted_command: InterpretedCommand,
) -> protocol::DacResponse {
    // Handle the interpreted command and create a response.
    match interpreted_command {
        // If the command was known, process it and update the DAC state.
        InterpretedCommand::Known { command } => process_command(dac, state, command),
        // If the command was unknown, reply with a NAK - Invalid.
        InterpretedCommand::Unknown { start_byte } => {
            let dac_status = dac.status.to_protocol();
            let response = protocol::DacResponse::NAK_INVALID;
            let command = start_byte;
            protocol::DacResponse {
                response,
                command,
                dac_status,
            }
        }
    }
}

/// Process the given the given **Command** and update the DAC state accordingly.
pub fn process_command(
    dac: &mut dac::Addressed,
    state: &mut State,
    command: Command,
) -> protocol::DacResponse {
    let (response, command) = match command {
        // Prepare the stream for playback.
        Command::PrepareStream(_prepare_stream) => {
            let response = match (dac.status.light_engine, dac.status.playback) {
                // Light engine must be `Ready` and playback must be `Idle`.
                (dac::LightEngine::Ready, dac::Playback::Idle) => {
                    // Update the state of the DAC.
                    dac.status.playback = dac::Playback::Prepared;
                    dac.status.point_count = 0;
                    dac.status.buffer_fullness = 0;
                    state.buffer.clear();
                    state.next_point_emission = None;
                    protocol::DacResponse::ACK
                }
                // Otherwise, reply with NAK - Invalid.
                _unexpected_state => protocol::DacResponse::NAK_INVALID,
            };
            let command = command::PrepareStream::START_BYTE;
            (response, command)
        }

        // Start processing the points at the specified rate.
        Command::Begin(begin) => {
            let response = match (dac.status.light_engine, dac.status.playback) {
                (dac::LightEngine::Ready, dac::Playback::Prepared) => {
                    if !state.buffer.is_empty() {
                        dac.status.playback = dac::Playback::Playing;
                        dac.status.point_rate = begin.point_rate;
                        protocol::DacResponse::ACK
                    } else {
                        protocol::DacResponse::NAK_INVALID
                    }
                }
                _unexpected_state => protocol::DacResponse::NAK_INVALID,
            };
            let command = command::Begin::START_BYTE;
            (response, command)
        }

        // Enqueue the new point rate.
        Command::PointRate(point_rate) => {
            let response = match (dac.status.light_engine, dac.status.playback) {
                (dac::LightEngine::Ready, dac::Playback::Prepared)
                | (dac::LightEngine::Ready, dac::Playback::Playing) => {
                    dac.status.point_rate = point_rate.0;
                    // TODO: If point rate buffer is full, respond with NAK - FULL.
                    protocol::DacResponse::ACK
                }
                _unexpected_state => protocol::DacResponse::NAK_INVALID,
            };
            let command = command::PointRate::START_BYTE;
            (response, command)
        }

        // Received new point data for processing.
        Command::Data(command::Data { points }) => {
            let response = match (dac.status.light_engine, dac.status.playback) {
                (dac::LightEngine::Ready, dac::Playback::Prepared)
                | (dac::LightEngine::Ready, dac::Playback::Playing) => {
                    let current_len = state.buffer.len();
                    let target_len = current_len + points.len();
                    let new_len = std::cmp::min(target_len, dac.buffer_capacity as usize);
                    state
                        .buffer
                        .extend(points.iter().take(new_len - current_len));
                    let response = if target_len <= dac.buffer_capacity as usize {
                        protocol::DacResponse::ACK
                    } else {
                        protocol::DacResponse::NAK_FULL
                    };
                    dac.status.buffer_fullness = state.buffer.len() as u16;
                    response
                }
                _unexpected_state => protocol::DacResponse::NAK_INVALID,
            };
            let command = command::Data::START_BYTE;
            (response, command)
        }

        // Stop processing points.
        Command::Stop(_stop) => {
            let response = match (dac.status.light_engine, dac.status.playback) {
                (dac::LightEngine::Ready, dac::Playback::Prepared)
                | (dac::LightEngine::Ready, dac::Playback::Playing) => {
                    dac.status.playback = dac::Playback::Idle;
                    protocol::DacResponse::ACK
                }
                _unexpected_state => protocol::DacResponse::NAK_INVALID,
            };
            let command = command::Stop::START_BYTE;
            (response, command)
        }

        // Immediately stop processing points and switch the LightEngine to ESTOP state.
        Command::EmergencyStop(_e_stop) => {
            dac.status.light_engine = dac::LightEngine::EmergencyStop;
            dac.status.playback = dac::Playback::Idle;
            let response = protocol::DacResponse::ACK;
            let command = command::EmergencyStop::START_BYTE;
            (response, command)
        }

        // Reset the light engine emergency stop to ready state.
        Command::ClearEmergencyStop(_e_stop) => {
            // TODO: Emit `NAK - Stop Condition if estop is still on?
            let response = match dac.status.light_engine {
                dac::LightEngine::Ready => {
                    dac.status.light_engine = dac::LightEngine::Ready;
                    protocol::DacResponse::ACK
                }
                _unexpected_state => protocol::DacResponse::NAK_INVALID,
            };
            let command = command::ClearEmergencyStop::START_BYTE;
            (response, command)
        }

        // Always respond to pings with ACK packets.
        Command::Ping(_ping) => {
            let response = protocol::DacResponse::ACK;
            let command = command::Ping::START_BYTE;
            (response, command)
        }
    };
    let dac_status = dac.status.to_protocol();
    protocol::DacResponse {
        response,
        command,
        dac_status,
    }
}

/// Convert the rate in hz to the periodic interval duration.
fn rate_interval(hz: u32) -> std::time::Duration {
    let secsf = 1.0 / hz as f64;
    let secs = secsf as u64;
    let nanos = (1_000_000_000.0 * (secsf - secs as f64)) as u32;
    std::time::Duration::new(secs, nanos)
}
