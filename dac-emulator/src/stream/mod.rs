//! The TCP stream between a client and the DAC.

mod output;

use ether_dream::dac;
use ether_dream::protocol::{self, command, Command as CommandTrait, ReadBytes, SizeBytes, WriteBytes};
use std::io::{self, Read, Write};
use std::{net, thread};

pub use self::output::Output;

/// A stream of communication between a DAC and a user.
///
/// The stream expects to receive **Command**s and responseds with **DacResponse**s.
///
/// All communication occurs over a single TCP stream.
///
/// Processed data is submitted to the **stream::Output** with which the user may do what they
/// like.
pub struct Stream {
    dac: dac::Addressed,
    tcp_stream: net::TcpStream,
    bytes: Box<[u8]>,
    output_processor: output::Handle,
}

/// A handle to the threads that make up a user/DAC communication stream.
pub struct Handle {
    tcp_stream: net::TcpStream,
    stream_thread: thread::JoinHandle<(dac::Addressed, io::Error)>,
    output: Output,
}

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
    Unknown { start_byte: u8 }
}

impl Handle {
    /// Produce a handle to the **Output** of the stream.
    ///
    /// The returned **Output** yields **Frame**s of **DacPoint**s which may be used for debugging
    /// or for visualisation.
    pub fn output(&self) -> Output {
        self.output.clone()
    }

    /// Wait for the DAC to finish communicating with the stream and return its resulting state.
    pub fn wait(self) -> (dac::Addressed, io::Error) {
        let Handle { stream_thread, ..  } = self;
        let result = stream_thread.join().expect("failed to join stream thread");
        result
    }

    /// Force the TCP connection to close right now and return the resulting state of the DAC.
    pub fn close(self) -> (dac::Addressed, io::Error) {
        self.tcp_stream.shutdown(net::Shutdown::Both).ok();
        self.wait()
    }
}

impl Stream {
    /// Initialise a new **Stream**.
    ///
    /// Internally this allocates a buffer of bytes whose size is the size of the largest possible
    /// **Data** command that may be received based on the DAC's buffer capacity.
    ///
    /// This function also spawns a thread used for processing output.
    pub fn new(
        dac: dac::Addressed,
        tcp_stream: net::TcpStream,
        output_frame_rate: u32,
    ) -> io::Result<Self>
    {
        // Create and spawn the output processor on its own thread.
        let output_processor = output::Processor::new(output_frame_rate).spawn()?;

        // Prepare a buffer with the maximum expected command size.
        let bytes: Box<[u8]> = {
            let data_command_size_bytes = 1;
            let data_len_size_bytes = 2;
            let max_points_size_bytes = dac.buffer_capacity as usize * protocol::DacPoint::SIZE_BYTES;
            let max_command_size = data_command_size_bytes
                + data_len_size_bytes
                + max_points_size_bytes;
            vec![0u8; max_command_size].into()
        };

        Ok(Stream {
            dac,
            tcp_stream,
            bytes,
            output_processor,
        })
    }

    /// Handle TCP messages received on the given stream by attempting to interpret them as commands.
    ///
    /// Once processed, each command will be responded to.
    ///
    /// This runs forever until either an error occurs or the TCP stream is shutdown.
    ///
    /// Returns the **io::Error** that caused the loop to end.
    pub fn run(&mut self) -> io::Error {
        // Loop reading and responding to a command at a time.
        let err = loop {
            match read_command_via_tcp_and_respond(self) {
                Ok(()) => continue,
                Err(err) => break err,
            }
        };

        // Stop the output processor thread. The thread will auto close when the stream is dropped.
        self.output_processor.stop();

        // Ensure the TCP stream is shutdown before exiting the thread.
        self.tcp_stream.shutdown(net::Shutdown::Both).ok();

        err
    }

    /// Spawn a stream that receives **Command**s sent by the user via the given TCP stream, processes
    /// them, updates the DAC state accordingly and responds via the TCP stream.
    ///
    /// Returns a **Handle** to the thread.
    pub fn spawn(mut self) -> io::Result<Handle> {
        let output = self.output_processor.output();
        let tcp_stream = self.tcp_stream.try_clone()?;
        let stream_thread = thread::Builder::new()
            .name("ether-dream-dac-emulator-stream".into())
            .spawn(move || {
                let io_err = self.run();
                let Stream { dac, output_processor, .. } = self;
                output_processor.close();
                (dac, io_err)
            })?;
        let handle = Handle { stream_thread, tcp_stream, output };
        Ok(handle)
    }
}

impl InterpretedCommand {
    /// Read a single command from the TCP stream and return it.
    ///
    /// This method blocks until the exact number of bytes necessary for the returned command are
    /// read.
    pub fn read_from_tcp_stream(
        bytes: &mut [u8],
        tcp_stream: &mut net::TcpStream,
    ) -> io::Result<Self>
    {
        // Peek the first byte to determine the command kind.
        bytes[0] = 0u8;
        let len = tcp_stream.peek(&mut bytes[..1])?;

        // Empty messages should only happen if the stream has closed.
        if len == 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "read `0` bytes from tcp stream"));
        }

        // Read the rest of the command from the stream based on the starting byte.
        let interpreted_command = match bytes[0] {
            command::PrepareStream::START_BYTE => {
                tcp_stream.read_exact(&mut bytes[..command::PrepareStream::SIZE_BYTES])?;
                let prepare_stream = (&bytes[..]).read_bytes::<command::PrepareStream>()?;
                Command::PrepareStream(prepare_stream).into()
            },
            command::Begin::START_BYTE => {
                tcp_stream.read_exact(&mut bytes[..command::Begin::SIZE_BYTES])?;
                let begin = (&bytes[..]).read_bytes::<command::Begin>()?;
                Command::Begin(begin).into()
            },
            command::PointRate::START_BYTE => {
                tcp_stream.read_exact(&mut bytes[..command::PointRate::SIZE_BYTES])?;
                let point_rate = (&bytes[..]).read_bytes::<command::PointRate>()?;
                Command::PointRate(point_rate).into()
            },
            command::Data::START_BYTE => {
                // Read the number of points.
                let command_bytes = 1;
                let n_points_bytes = 2;
                let n_points_start = command_bytes;
                let n_points_end = n_points_start + n_points_bytes;
                tcp_stream.peek(&mut bytes[..n_points_end])?;
                let n_points = command::Data::read_n_points(&bytes[n_points_start..n_points_end])?;

                // Use the number of points to determine how many bytes to read.
                let data_bytes = n_points as usize * protocol::DacPoint::SIZE_BYTES;
                let total_bytes = command_bytes + n_points_bytes + data_bytes;
                tcp_stream.read_exact(&mut bytes[..total_bytes])?;
                let data = (&bytes[..]).read_bytes::<command::Data<'static>>()?;
                Command::Data(data).into()
            },
            command::Stop::START_BYTE => {
                tcp_stream.read_exact(&mut bytes[..command::Stop::SIZE_BYTES])?;
                let stop = (&bytes[..]).read_bytes::<command::Stop>()?;
                Command::Stop(stop).into()
            },
            command::EmergencyStop::START_BYTE => {
                tcp_stream.read_exact(&mut bytes[..command::EmergencyStop::SIZE_BYTES])?;
                let emergency_stop = (&bytes[..]).read_bytes::<command::EmergencyStop>()?;
                Command::EmergencyStop(emergency_stop).into()
            },
            command::ClearEmergencyStop::START_BYTE => {
                tcp_stream.read_exact(&mut bytes[..command::ClearEmergencyStop::SIZE_BYTES])?;
                let clear_emergency_stop = (&bytes[..]).read_bytes::<command::ClearEmergencyStop>()?;
                Command::ClearEmergencyStop(clear_emergency_stop).into()
            },
            command::Ping::START_BYTE => {
                tcp_stream.read_exact(&mut bytes[..command::Ping::SIZE_BYTES])?;
                let ping = (&bytes[..]).read_bytes::<command::Ping>()?;
                Command::Ping(ping).into()
            },
            start_byte => {
                InterpretedCommand::Unknown { start_byte }
            },
        };

        Ok(interpreted_command)
    }
}

impl From<Command> for InterpretedCommand {
    fn from(command: Command) -> Self {
        InterpretedCommand::Known { command }
    }
}

/// Read a single command from the TCP stream and respond.
///
/// If a `read` occurred of `0` bytes, this function returns early and no response is sent.
fn read_command_via_tcp_and_respond(stream: &mut Stream) -> io::Result<()> {
    let Stream {
        ref mut dac,
        ref mut tcp_stream,
        ref mut bytes,
        ref output_processor,
    } = *stream;

    // Read the command from the TCP stream.
    let interpreted_command = InterpretedCommand::read_from_tcp_stream(bytes, tcp_stream)?;

    // Process command here.
    let dac_response = process_interpreted_command(dac, interpreted_command, output_processor);

    // Write the response to bytes.
    let response_len = protocol::DacResponse::SIZE_BYTES;
    (&mut bytes[..response_len]).write_bytes(&dac_response)?;
    tcp_stream.write(&bytes[..response_len])?;

    Ok(())
}

/// Process the interpreted command, update the DAC state accordingly and produce a response.
pub fn process_interpreted_command(
    dac: &mut dac::Addressed,
    interpreted_command: InterpretedCommand,
    output_processor: &output::Handle,
) -> protocol::DacResponse
{
    // Handle the interpreted command and create a response.
    match interpreted_command {
        // If the command was known, process it and update the DAC state.
        InterpretedCommand::Known { command } => {
            process_command(dac, command, output_processor)
        },
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
    command: Command,
    output_processor: &output::Handle,
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
                    protocol::DacResponse::ACK
                },
                // Otherwise, reply with NAK - Invalid.
                _unexpected_state => {
                    protocol::DacResponse::NAK_INVALID
                },
            };
            let command = command::PrepareStream::START_BYTE;
            (response, command)
        },

        // Start processing the points at the specified rate.
        Command::Begin(begin) => {
            let response = match (dac.status.light_engine, dac.status.playback) {
                (dac::LightEngine::Ready, dac::Playback::Prepared) => {
                    if output_processor.buffer_fullness() > 0 {
                        output_processor.begin(begin);
                        dac.status.playback = dac::Playback::Playing;
                        dac.status.point_rate = begin.point_rate;
                        protocol::DacResponse::ACK
                    } else {
                        protocol::DacResponse::NAK_INVALID
                    }
                },
                _unexpected_state => {
                    protocol::DacResponse::NAK_INVALID
                },
            };
            let command = command::Begin::START_BYTE;
            (response, command)
        },

        // Enqueue the new point rate.
        Command::PointRate(point_rate) => {
            let response = match (dac.status.light_engine, dac.status.playback) {
                (dac::LightEngine::Ready, dac::Playback::Prepared) |
                (dac::LightEngine::Ready, dac::Playback::Playing) => {
                    output_processor.push_point_rate(point_rate);
                    // TODO: If point rate buffer is full, respond with NAK - FULL.
                    protocol::DacResponse::ACK
                },
                _unexpected_state => {
                    protocol::DacResponse::NAK_INVALID
                },
            };
            let command = command::PointRate::START_BYTE;
            (response, command)
        },

        // Received new point data for processing.
        Command::Data(command::Data { points }) => {
            let response = match (dac.status.light_engine, dac.status.playback) {
                (dac::LightEngine::Ready, dac::Playback::Prepared) |
                (dac::LightEngine::Ready, dac::Playback::Playing) => {
                    let mut points = points.into_owned();
                    let current_len = output_processor.buffer_fullness();
                    let new_len = current_len + points.len();
                    let response = if new_len < dac.buffer_capacity as usize {
                        protocol::DacResponse::ACK
                    } else {
                        points.truncate(dac.buffer_capacity as usize - current_len);
                        protocol::DacResponse::NAK_FULL
                    };
                    dac.status.buffer_fullness = output_processor.push_data(points) as _;
                    dac.status.point_count = output_processor.point_count() as _;
                    response
                },
                _unexpected_state => {
                    protocol::DacResponse::NAK_INVALID
                },
            };
            let command = command::Data::START_BYTE;
            (response, command)
        },

        // Stop processing points.
        Command::Stop(_stop) => {
            let response = match (dac.status.light_engine, dac.status.playback) {
                (dac::LightEngine::Ready, dac::Playback::Prepared) |
                (dac::LightEngine::Ready, dac::Playback::Playing) => {
                    output_processor.stop();
                    dac.status.playback = dac::Playback::Idle;
                    protocol::DacResponse::ACK
                },
                _unexpected_state => {
                    protocol::DacResponse::NAK_INVALID
                },
            };
            let command = command::Stop::START_BYTE;
            (response, command)
        },

        // Immediately stop processing points and switch the LightEngine to ESTOP state.
        Command::EmergencyStop(_e_stop) => {
            output_processor.stop();
            dac.status.light_engine = dac::LightEngine::EmergencyStop;
            dac.status.playback = dac::Playback::Idle;
            let response = protocol::DacResponse::ACK;
            let command = command::EmergencyStop::START_BYTE;
            (response, command)
        },

        // Reset the light engine emergency stop to ready state.
        Command::ClearEmergencyStop(_e_stop) => {
            // TODO: Emit `NAK - Stop Condition if estop is still on?
            let response = match dac.status.light_engine {
                dac::LightEngine::Ready => {
                    dac.status.light_engine = dac::LightEngine::Ready;
                    protocol::DacResponse::ACK
                },
                _unexpected_state => {
                    protocol::DacResponse::NAK_INVALID
                },
            };
            let command = command::ClearEmergencyStop::START_BYTE;
            (response, command)
        },

        // Always respond to pings with ACK packets.
        Command::Ping(_ping) => {
            let response = protocol::DacResponse::ACK;
            let command = command::Ping::START_BYTE;
            (response, command)
        },
    };
    let dac_status = dac.status.to_protocol();
    protocol::DacResponse {
        response,
        command,
        dac_status,
    }
}
