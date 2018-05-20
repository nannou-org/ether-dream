//! Types and constants that precisely match the specification.

use byteorder::{LE, ReadBytesExt, WriteBytesExt};
use std::io;

pub use self::command::Command;

/// Communication with the DAC happens over TCP on port 7765.
pub const COMMUNICATION_PORT: u16 = 7765;

/// The DAC sends UDP broadcast messages on port 7654.
///
/// This does not appeared to be documented in the protocol, but was found within the
/// `github.com/j4cbo/j4cDAC` repository.
pub const BROADCAST_PORT: u16 = 7654;

/// A trait for writing any of the Ether Dream protocol types to little-endian bytes.
///
/// A blanket implementation is provided for all types that implement `byteorder::WriteBytesExt`.
pub trait WriteBytes {
    fn write_bytes<P: WriteToBytes>(&mut self, protocol: P) -> io::Result<()>;
}

/// A trait for reading any of the Ether Dream protocol types from little-endian bytes.
///
/// A blanket implementation is provided for all types that implement `byteorder::ReadBytesExt`.
pub trait ReadBytes {
    fn read_bytes<P: ReadFromBytes>(&mut self) -> io::Result<P>;
}

/// Protocol types that may be written to little endian bytes.
pub trait WriteToBytes {
    /// Write the command to bytes.
    fn write_to_bytes<W: WriteBytesExt>(&self, W) -> io::Result<()>;
}

/// Protocol types that may be read from little endian bytes.
pub trait ReadFromBytes: Sized {
    /// Read the command from bytes.
    fn read_from_bytes<R: ReadBytesExt>(R) -> io::Result<Self>;
}

/// Types that have a constant size when written to or read from bytes.
pub trait SizeBytes {
    const SIZE_BYTES: usize;
}

/// Periodically, and as part of ACK packets, the DAC sends its current playback status to the
/// host.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct DacStatus {
    /// This remains undocumented in the protocol.
    ///
    /// The original implementation source simply sets this to `0`.
    pub protocol: u8,
    /// The current state of the "light engine" state machine.
    pub light_engine_state: u8,
    /// The current state of the "playback" state machine.
    pub playback_state: u8,
    /// The currently-selected data source:
    ///
    /// - `0`: Network streaming (the protocol implemented in this library).
    /// - `1`: ILDA playback from SD card.
    /// - `2`: Internal abstract generator.
    pub source: u8,
    /// If the light engine is `Ready`, this will be `0`.
    ///
    /// Otherwise, bits will be set as follows:
    ///
    /// - `0`: Emergency stop occurred due to E-Stop packet or invalid command.
    /// - `1`: Emergency stop occurred due to E-Stop input to projector.
    /// - `2`: Emergency stop input to projector is currently active.
    /// - `3`: Emergency stop occurred due to over-temperature condition.
    /// - `4`: Over-temperature condition is currently active.
    /// - `5`: Emergency stop occurred due to loss of ethernet link.
    /// 
    /// All remaining are reserved for future use.
    pub light_engine_flags: u16,
    /// These flags may be non-zero during normal operation.
    ///
    /// Bits are defined as follows:
    ///
    /// - `0`: **Shutter state**. `0` is closed, `1` is open.
    /// - `1`: **Underflow**. `1` if the last stream ended with underflow rather than a `Stop`
    ///   command. This is reset to `0` by the `Prepare` command.
    /// - `2`: **E-Stop**. `1` if the last stream ended because the E-Stop state was entered. Reset
    ///   to zero by the `Prepare` command.
    pub playback_flags: u16,
    /// This field is undocumented within the protocol reference.
    ///
    /// By looking at the source code of the original implementation, this seems to represent the
    /// state of the current `source`.
    ///
    /// If `source` is set to `1` for ILDA playback from SD card, the following flags are defined:
    ///
    /// - `0`: `ILDA_PLAYER_PLAYING`.
    /// - `1`: `ILDA_PLAYER_REPEAT`.
    ///
    /// If `source` is set to `2` for the internal abstract generator, the flags are defined as
    /// follows:
    ///
    /// - `0`: `ABSTRACT_PLAYING`.
    pub source_flags: u16,
    /// The number of points currently buffered.
    pub buffer_fullness: u16,
    /// If in the `Prepared` or `Playing` playback states, this is the number of points per
    /// second for which the DAC is configured.
    ///
    /// If in the `Idle` playback state, this will be `0`.
    pub point_rate: u32,
    /// If in the `Playing` playback state, this is the number of points that the DAC has actually
    /// emitted since it started playing.
    ///
    /// If in the `Prepared` or `Idle` playback states, this will be `0`.
    pub point_count: u32,
}

/// Regardless of the data source being used, each DAC broadcasts a status/ID datagram over UDP to
/// its local network's broadcast address once per second.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct DacBroadcast {
    /// The unique hardware identifier for the DAC.
    pub mac_address: [u8; 6],
    /// This field is undocumented in the official protocol but seems to represent a version number
    /// for the hardware in use by the DAC.
    pub hw_revision: u16,
    /// This field is undocumented in the official protocol but seems to represent the version of
    /// the protocol implementation. As of writing this, this is hardcoded as `2` in the original
    /// source.
    pub sw_revision: u16,
    /// The DAC's maximum buffer capacity for storing points that are yet to be converted to
    /// output.
    ///
    /// As of writing this, this is hardcoded to `1800` in the original DAC source code.
    pub buffer_capacity: u16,
    /// The DAC's maximum point rate.
    ///
    /// As of writing this, this is hardcoded to `100_000` in the original DAC source code.
    pub max_point_rate: u32,
    /// The current status of the DAC.
    pub dac_status: DacStatus,
}

/// Values are full-scale.
///
/// E.g. for all color channels, `65535` is full output while `0` is no output.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct DacPoint {
    /// A set of bit fields. The following fields are defined:
    ///
    /// - `15`: Change point rate. If this bit is set and there are any values in the point
    ///   rate change buffer a new rate is read out of the buffer and set as the current
    ///   playback rate. If the buffer is empty, the point rate is not changed.
    ///
    /// All other bits are reserved for future expansion to support extra TTL outputs, etc.
    pub control: u16,
    /// -32768 is the start along the *x* axis (left-most point).
    /// 32767 is the end along the *x* axis (right-most point).
    pub x: i16,
    /// -32768 is the start along the *y* axis (bottom-most point).
    /// 32767 is the end along the *y* axis (top-most point).
    pub y: i16,
    pub i: u16,
    /// `0` is no red. `65535` is full red.
    pub r: u16,
    /// `0` is no green. `65535` is full green.
    pub g: u16,
    /// `0` is no blue. `65535` is full blue.
    pub b: u16,
    pub u1: u16,
    pub u2: u16,
}

/// A response from a DAC.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct DacResponse {
    /// See the `DacResponse` associated constants for the possible values for this field.
    pub response: u8,
    /// In the case of ACK/NAK responses, this echoes back the command to which the response is
    /// sent.
    ///
    /// Commands are always sent in order, so this field exists for sanity-checking on the
    /// host-side.
    pub command: u8,
    /// The current status of the DAC.
    pub dac_status: DacStatus,
}

impl DacStatus {
    /// The light engine is ready.
    pub const LIGHT_ENGINE_READY: u8 = 0;
    /// In the case where the DAC is also used for thermal control of laser apparatus, this is the
    /// state that is entered after power-up.
    pub const LIGHT_ENGINE_WARMUP: u8 = 1;
    /// Lasers are off but thermal control is still active.
    pub const LIGHT_ENGINE_COOLDOWN: u8 = 2;
    /// An emergency stop has been triggered, either by an E-stop input on the DAC, an E-stop
    /// command over the network, or a fault such as over-temperature.
    pub const LIGHT_ENGINE_EMERGENCY_STOP: u8 = 3;

    /// The default state:
    ///
    /// - No points may be added to the buffer.
    /// - No output is generated.
    /// - All analog outputs are at 0v.
    /// - The shutter is controlled by the data source.
    pub const PLAYBACK_IDLE: u8 = 0;
    /// The buffer will accept points.
    ///
    /// The output is the same as the `Idle` state
    pub const PLAYBACK_PREPARED: u8 = 1;
    /// Points are being sent to the output.
    pub const PLAYBACK_PLAYING: u8 = 2;

    /// Network streaming (the protocol implemented in this library).
    pub const SOURCE_NETWORK_STREAMING: u8 = 0;
    /// ILDA playback from SD card.
    pub const SOURCE_ILDA_PLAYBACK_SD: u8 = 1;
    /// Internal abstract generator.
    pub const SOURCE_INTERNAL_ABSTRACT_GENERATOR: u8 = 2;
}

impl DacResponse {
    /// The previous command was accepted.
    pub const ACK: u8 = 0x61;
    /// The write command could not be performed because there was not enough buffer space when it
    /// was received.
    pub const NAK_FULL: u8 = 0x46;
    /// The command contained an invalid `command` byte or parameters.
    pub const NAK_INVALID: u8 = 0x49;
    /// An emergency-stop condition still exists.
    pub const NAK_STOP_CONDITION: u8 = 0x21;
}

impl WriteToBytes for DacStatus {
    fn write_to_bytes<W: WriteBytesExt>(&self, mut writer: W) -> io::Result<()> {
        writer.write_u8(self.protocol)?;
        writer.write_u8(self.light_engine_state)?;
        writer.write_u8(self.playback_state)?;
        writer.write_u8(self.source)?;
        writer.write_u16::<LE>(self.light_engine_flags)?;
        writer.write_u16::<LE>(self.playback_flags)?;
        writer.write_u16::<LE>(self.source_flags)?;
        writer.write_u16::<LE>(self.buffer_fullness)?;
        writer.write_u32::<LE>(self.point_rate)?;
        writer.write_u32::<LE>(self.point_count)?;
        Ok(())
    }
}

impl WriteToBytes for DacBroadcast {
    fn write_to_bytes<W: WriteBytesExt>(&self, mut writer: W) -> io::Result<()> {
        for &byte in &self.mac_address {
            writer.write_u8(byte)?;
        }
        writer.write_u16::<LE>(self.hw_revision)?;
        writer.write_u16::<LE>(self.sw_revision)?;
        writer.write_u16::<LE>(self.buffer_capacity)?;
        writer.write_u32::<LE>(self.max_point_rate)?;
        writer.write_bytes(&self.dac_status)?;
        Ok(())
    }
}

impl WriteToBytes for DacPoint {
    fn write_to_bytes<W: WriteBytesExt>(&self, mut writer: W) -> io::Result<()> {
        writer.write_u16::<LE>(self.control)?;
        writer.write_i16::<LE>(self.x)?;
        writer.write_i16::<LE>(self.y)?;
        writer.write_u16::<LE>(self.i)?;
        writer.write_u16::<LE>(self.r)?;
        writer.write_u16::<LE>(self.g)?;
        writer.write_u16::<LE>(self.b)?;
        writer.write_u16::<LE>(self.u1)?;
        writer.write_u16::<LE>(self.u2)?;
        Ok(())
    }
}

impl WriteToBytes for DacResponse {
    fn write_to_bytes<W: WriteBytesExt>(&self, mut writer: W) -> io::Result<()> {
        writer.write_u8(self.response)?;
        writer.write_u8(self.command)?;
        writer.write_bytes(&self.dac_status)?;
        Ok(())
    }
}

impl ReadFromBytes for DacStatus {
    fn read_from_bytes<R: ReadBytesExt>(mut reader: R) -> io::Result<Self> {
        let protocol = reader.read_u8()?;
        let light_engine_state = reader.read_u8()?;
        let playback_state = reader.read_u8()?;
        let source = reader.read_u8()?;
        let light_engine_flags = reader.read_u16::<LE>()?;
        let playback_flags = reader.read_u16::<LE>()?;
        let source_flags = reader.read_u16::<LE>()?;
        let buffer_fullness = reader.read_u16::<LE>()?;
        let point_rate = reader.read_u32::<LE>()?;
        let point_count = reader.read_u32::<LE>()?;
        let dac_status = DacStatus {
            protocol,
            light_engine_state,
            playback_state,
            source,
            light_engine_flags,
            playback_flags,
            source_flags,
            buffer_fullness,
            point_rate,
            point_count,
        };
        Ok(dac_status)
    }
}

impl ReadFromBytes for DacBroadcast {
    fn read_from_bytes<R: ReadBytesExt>(mut reader: R) -> io::Result<Self> {
        let mac_address = [
            reader.read_u8()?,
            reader.read_u8()?,
            reader.read_u8()?,
            reader.read_u8()?,
            reader.read_u8()?,
            reader.read_u8()?,
        ];
        let hw_revision = reader.read_u16::<LE>()?;
        let sw_revision = reader.read_u16::<LE>()?;
        let buffer_capacity = reader.read_u16::<LE>()?;
        let max_point_rate = reader.read_u32::<LE>()?;
        let dac_status = reader.read_bytes::<DacStatus>()?;
        let dac_broadcast = DacBroadcast {
            mac_address,
            hw_revision,
            sw_revision,
            buffer_capacity,
            max_point_rate,
            dac_status,
        };
        Ok(dac_broadcast)
    }
}

impl ReadFromBytes for DacPoint {
    fn read_from_bytes<R: ReadBytesExt>(mut reader: R) -> io::Result<Self> {
        let control = reader.read_u16::<LE>()?;
        let x = reader.read_i16::<LE>()?;
        let y = reader.read_i16::<LE>()?;
        let i = reader.read_u16::<LE>()?;
        let r = reader.read_u16::<LE>()?;
        let g = reader.read_u16::<LE>()?;
        let b = reader.read_u16::<LE>()?;
        let u1 = reader.read_u16::<LE>()?;
        let u2 = reader.read_u16::<LE>()?;
        let dac_point = DacPoint {
            control,
            x,
            y,
            i,
            r,
            g,
            b,
            u1,
            u2,
        };
        Ok(dac_point)
    }
}

impl ReadFromBytes for DacResponse {
    fn read_from_bytes<R: ReadBytesExt>(mut reader: R) -> io::Result<Self> {
        let response = reader.read_u8()?;
        let command = reader.read_u8()?;
        let dac_status = reader.read_bytes::<DacStatus>()?;
        let dac_response = DacResponse {
            response,
            command,
            dac_status,
        };
        Ok(dac_response)
    }
}

impl SizeBytes for DacStatus {
    const SIZE_BYTES: usize = 20;
}

impl SizeBytes for DacBroadcast {
    const SIZE_BYTES: usize = DacStatus::SIZE_BYTES + 16;
}

impl SizeBytes for DacPoint {
    const SIZE_BYTES: usize = 18;
}

impl SizeBytes for DacResponse {
    const SIZE_BYTES: usize = DacStatus::SIZE_BYTES + 2;
}

impl<'a, P> WriteToBytes for &'a P
where
    P: WriteToBytes,
{
    fn write_to_bytes<W: WriteBytesExt>(&self, writer: W) -> io::Result<()> {
        (*self).write_to_bytes(writer)
    }
}

impl<W> WriteBytes for W
where
    W: WriteBytesExt,
{
    fn write_bytes<P: WriteToBytes>(&mut self, protocol: P) -> io::Result<()> {
        protocol.write_to_bytes(self)
    }
}

impl<R> ReadBytes for R
where
    R: ReadBytesExt,
{
    fn read_bytes<P: ReadFromBytes>(&mut self) -> io::Result<P> {
        P::read_from_bytes(self)
    }
}

/// When a host first connects to the device, the device immediately sends it a status reply as if
/// the host had sent a ping packet. The host sends to the device a series of commands. All
/// commands receive a response from the DAC.
pub mod command {
    use byteorder::{LE, ReadBytesExt, WriteBytesExt};
    use std::borrow::Cow;
    use std::{self, io};
    use super::{DacPoint, ReadBytes, ReadFromBytes, SizeBytes, WriteBytes, WriteToBytes};

    /// Types that may be submitted as commands to the DAC.
    pub trait Command {
        /// The starting byte of the command.
        const START_BYTE: u8;
        /// A provided method for producing the start byte. Useful for trait objects.
        fn start_byte(&self) -> u8 {
            Self::START_BYTE
        }
    }

    /// This command causes the playback system to enter the `Prepared` state. The DAC resets its
    /// buffer to be empty and sets "point_count" to `0`.
    ///
    /// This command may only be sent if the light engine is `Ready` and the playback system is
    /// `Idle`. If so, the DAC replies with ACK. Otherwise, it replies with NAK - Invalid.
    #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
    pub struct PrepareStream;

    /// Causes the DAC to begin producing output.
    ///
    /// If the playback system was `Prepared` and there was data in the buffer, then the DAC will
    /// reply with ACK.
    ///
    /// Otherwise, it replies with NAK - Invalid.
    #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
    pub struct Begin {
        /// *Currently unused.*
        pub low_water_mark: u16,
        /// The number of points per second to be read from the buffer.
        pub point_rate: u32,
    }

    /// Adds a new point rate (in points per second) to the point rate buffer.
    ///
    /// Point rate changes are read out of the buffer when a point with an appropriate flag is
    /// played (see the `WriteData` command).
    ///
    /// If the DAC's playback state is not `Prepared` or `Playing`, it replies with NAK - Invalid.
    /// 
    /// If the point rate buffer is full, it replies with NAK - Full.
    ///
    /// Otherwise, it replies with ACK.
    #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
    pub struct PointRate(pub u32);

    /// Indicates to the DAC to add the following point data into its buffer.
    ///
    /// Point data is laid out as follows:
    ///
    /// - 0x64
    /// - <the number of points>: u16
    /// - <point data>
    #[derive(Clone, Debug, PartialEq, Eq, Hash)]
    pub struct Data<'a> {
        pub points: Cow<'a, [DacPoint]>,
    }

    /// Causes the DAC to immediately stop playing and return to the `Idle` playback state.
    ///
    /// It is ACKed if the DAC was in the `Playing` or `Prepared` playback states.
    ///
    /// Otherwise it is replied to with NAK - Invalid.
    #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
    pub struct Stop;

    /// Causes the light engine to enter the E-Stop state, regardless of its previous state.
    ///
    /// This command is always ACKed.
    ///
    /// **Note:** Any unrecognised command will also be treated as E-Stop. However, software should
    /// not send undefined commands deliberately, since they may be defined in the future.
    #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
    pub struct EmergencyStop;

    /// Causes the light engine to enter the E-Stop state, regardless of its previous state.
    ///
    /// This command is always ACKed.
    ///
    /// **Note:** Any unrecognised command will also be treated as E-Stop. However, software should
    /// not send undefined commands deliberately, since they may be defined in the future.
    #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
    pub struct EmergencyStopAlt;

    /// If the light engine was in E-Stop state due to an emergency stop command (either from a
    /// local stop condition or over the network), this command resets it to `Ready`.
    ///
    /// It is ACKed if the DAC was previously in E-Stop.
    ///
    /// Otherwise it is replied to with a NAK - Invalid.
    ///
    /// If the condition that caused the emergency stop is still active (e.g. E-Stop input still
    /// asserted, temperature still out of bounds, etc) a NAK - Stop Condition is sent.
    #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
    pub struct ClearEmergencyStop;

    /// The DAC will reply to this with an ACK packet.
    ///
    /// This serves as a keep-alive for the connection when the DAC is not actively streaming.
    #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
    pub struct Ping;

    impl Begin {
        /// Consecutively read the fields of the **Begin** type and return a **Begin** instance.
        ///
        /// Note that if reading from a stream, this method assumes that the starting command byte
        /// has already been read.
        pub fn read_fields<R: ReadBytesExt>(mut reader: R) -> io::Result<Self> {
            let low_water_mark = reader.read_u16::<LE>()?;
            let point_rate = reader.read_u32::<LE>()?;
            let begin = Begin { low_water_mark, point_rate };
            Ok(begin)
        }
    }

    impl PointRate {
        /// Consecutively read the fields of the **PointRate** type and return a **PointRate** instance.
        ///
        /// Note that if reading from a stream, this method assumes that the starting command byte
        /// has already been read.
        pub fn read_fields<R: ReadBytesExt>(mut reader: R) -> io::Result<Self> {
            let point_rate = PointRate(reader.read_u32::<LE>()?);
            Ok(point_rate)
        }
    }

    impl<'a> Data<'a> {
        /// Read the `u16` representing the number of points within the **Data** from the given
        /// `reader`.
        ///
        /// This method is useful for determining how many more bytes should be read from a stream.
        ///
        /// Note that if reading from a stream, this method assumes that the starting command byte
        /// has already been read.
        pub fn read_n_points<R>(mut reader: R) -> io::Result<u16>
        where
            R: ReadBytesExt,
        {
            reader.read_u16::<LE>()
        }

        /// Read and append the given number of points into the given Vec of **DacPoint**s.
        ///
        /// This method is useful as an alternative to **Self::read_fields** or
        /// **Self::read_from_bytes** as it allows for re-using a buffer of points rather than
        /// dynamically allocating a new one each time.
        ///
        /// Note that if reading from a stream, this method assumes that the starting command byte
        /// and the `u16` representing the number of points have both already been read.
        pub fn read_points<R>(
            mut reader: R,
            mut n_points: u16,
            points: &mut Vec<DacPoint>,
        ) -> io::Result<()>
        where
            R: ReadBytesExt,
        {
            while n_points > 0 {
                let dac_point = reader.read_bytes::<DacPoint>()?;
                points.push(dac_point);
                n_points -= 1;
            }
            Ok(())
        }
    }

    impl Data<'static> {
        /// Consecutively read the fields of the **Data** type and return a **Data** instance.
        ///
        /// Note that if reading from a stream, this method assumes that the starting command byte
        /// has already been read.
        pub fn read_fields<R: ReadBytesExt>(mut reader: R) -> io::Result<Self> {
            let n_points = Self::read_n_points(&mut reader)?;
            let mut data = Vec::with_capacity(n_points as _);
            Self::read_points(reader, n_points, &mut data)?;
            let data = Data { points: Cow::Owned(data) };
            Ok(data)
        }
    }

    impl<'a, C> Command for &'a C
    where
        C: Command,
    {
        const START_BYTE: u8 = C::START_BYTE;
    }

    impl Command for PrepareStream {
        const START_BYTE: u8 = 0x70;
    }

    impl Command for Begin {
        const START_BYTE: u8 = 0x62;
    }

    impl Command for PointRate {
        const START_BYTE: u8 = 0x74;
    }

    impl<'a> Command for Data<'a> {
        const START_BYTE: u8 = 0x64;
    }

    impl Command for Stop {
        const START_BYTE: u8 = 0x73;
    }

    impl Command for EmergencyStop {
        const START_BYTE: u8 = 0x00;
    }

    impl Command for EmergencyStopAlt {
        const START_BYTE: u8 = 0xff;
    }

    impl Command for ClearEmergencyStop {
        const START_BYTE: u8 = 0x63;
    }

    impl Command for Ping {
        const START_BYTE: u8 = 0x3f;
    }

    impl SizeBytes for PrepareStream {
        const SIZE_BYTES: usize = 1;
    }

    impl SizeBytes for Begin {
        const SIZE_BYTES: usize = 7;
    }

    impl SizeBytes for PointRate {
        const SIZE_BYTES: usize = 5;
    }

    impl SizeBytes for Stop {
        const SIZE_BYTES: usize = 1;
    }

    impl SizeBytes for EmergencyStop {
        const SIZE_BYTES: usize = 1;
    }

    impl SizeBytes for ClearEmergencyStop {
        const SIZE_BYTES: usize = 1;
    }

    impl SizeBytes for Ping {
        const SIZE_BYTES: usize = 1;
    }

    impl WriteToBytes for PrepareStream {
        fn write_to_bytes<W: WriteBytesExt>(&self, mut writer: W) -> io::Result<()> {
            writer.write_u8(Self::START_BYTE)?;
            Ok(())
        }
    }

    impl WriteToBytes for Begin {
        fn write_to_bytes<W: WriteBytesExt>(&self, mut writer: W) -> io::Result<()> {
            writer.write_u8(Self::START_BYTE)?;
            writer.write_u16::<LE>(self.low_water_mark)?;
            writer.write_u32::<LE>(self.point_rate)?;
            Ok(())
        }
    }

    impl WriteToBytes for PointRate {
        fn write_to_bytes<W: WriteBytesExt>(&self, mut writer: W) -> io::Result<()> {
            writer.write_u8(Self::START_BYTE)?;
            writer.write_u32::<LE>(self.0)?;
            Ok(())
        }
    }

    impl<'a> WriteToBytes for Data<'a> {
        fn write_to_bytes<W: WriteBytesExt>(&self, mut writer: W) -> io::Result<()> {
            if self.points.len() > std::u16::MAX as usize {
                let err_msg = "the number of points exceeds the maximum possible `u16` value";
                return Err(io::Error::new(io::ErrorKind::InvalidData, err_msg));
            }
            writer.write_u8(Self::START_BYTE)?;
            writer.write_u16::<LE>(self.points.len() as u16)?;
            for point in self.points.iter() {
                writer.write_bytes(point)?;
            }
            Ok(())
        }
    }

    impl WriteToBytes for Stop {
        fn write_to_bytes<W: WriteBytesExt>(&self, mut writer: W) -> io::Result<()> {
            writer.write_u8(Self::START_BYTE)?;
            Ok(())
        }
    }

    impl WriteToBytes for EmergencyStop {
        fn write_to_bytes<W: WriteBytesExt>(&self, mut writer: W) -> io::Result<()> {
            writer.write_u8(Self::START_BYTE)?;
            Ok(())
        }
    }

    impl WriteToBytes for EmergencyStopAlt {
        fn write_to_bytes<W: WriteBytesExt>(&self, mut writer: W) -> io::Result<()> {
            writer.write_u8(Self::START_BYTE)?;
            Ok(())
        }
    }

    impl WriteToBytes for ClearEmergencyStop {
        fn write_to_bytes<W: WriteBytesExt>(&self, mut writer: W) -> io::Result<()> {
            writer.write_u8(Self::START_BYTE)?;
            Ok(())
        }
    }

    impl WriteToBytes for Ping {
        fn write_to_bytes<W: WriteBytesExt>(&self, mut writer: W) -> io::Result<()> {
            writer.write_u8(Self::START_BYTE)?;
            Ok(())
        }
    }

    impl ReadFromBytes for PrepareStream {
        fn read_from_bytes<R: ReadBytesExt>(mut reader: R) -> io::Result<Self> {
            let command = reader.read_u8()?;
            if command != Self::START_BYTE {
                let err_msg = "invalid \"prepare stream\" command byte";
                return Err(io::Error::new(io::ErrorKind::InvalidData, err_msg));
            }
            Ok(PrepareStream)
        }
    }

    impl ReadFromBytes for Begin {
        fn read_from_bytes<R: ReadBytesExt>(mut reader: R) -> io::Result<Self> {
            if reader.read_u8()? != Self::START_BYTE {
                let err_msg = "invalid \"begin\" command byte";
                return Err(io::Error::new(io::ErrorKind::InvalidData, err_msg));
            }
            Self::read_fields(reader)
        }
    }

    impl ReadFromBytes for PointRate {
        fn read_from_bytes<R: ReadBytesExt>(mut reader: R) -> io::Result<Self> {
            if reader.read_u8()? != Self::START_BYTE {
                let err_msg = "invalid \"queue change\" command byte";
                return Err(io::Error::new(io::ErrorKind::InvalidData, err_msg));
            }
            Self::read_fields(reader)
        }
    }

    impl ReadFromBytes for Data<'static> {
        fn read_from_bytes<R: ReadBytesExt>(mut reader: R) -> io::Result<Self> {
            if reader.read_u8()? != Self::START_BYTE {
                let err_msg = "invalid \"data\" command byte";
                return Err(io::Error::new(io::ErrorKind::InvalidData, err_msg));
            }
            Self::read_fields(reader)
        }
    }

    impl ReadFromBytes for Stop {
        fn read_from_bytes<R: ReadBytesExt>(mut reader: R) -> io::Result<Self> {
            let command = reader.read_u8()?;
            if command != Self::START_BYTE {
                let err_msg = "invalid \"stop\" command byte";
                return Err(io::Error::new(io::ErrorKind::InvalidData, err_msg));
            }
            Ok(Stop)
        }
    }

    impl ReadFromBytes for EmergencyStop {
        fn read_from_bytes<R: ReadBytesExt>(mut reader: R) -> io::Result<Self> {
            let command = reader.read_u8()?;
            if command != Self::START_BYTE && command != EmergencyStopAlt::START_BYTE {
                let err_msg = "invalid \"emergency stop\" command byte";
                return Err(io::Error::new(io::ErrorKind::InvalidData, err_msg));
            }
            Ok(EmergencyStop)
        }
    }

    impl ReadFromBytes for ClearEmergencyStop {
        fn read_from_bytes<R: ReadBytesExt>(mut reader: R) -> io::Result<Self> {
            let command = reader.read_u8()?;
            if command != Self::START_BYTE {
                let err_msg = "invalid \"clear emergency stop\" command byte";
                return Err(io::Error::new(io::ErrorKind::InvalidData, err_msg));
            }
            Ok(ClearEmergencyStop)
        }
    }

    impl ReadFromBytes for Ping {
        fn read_from_bytes<R: ReadBytesExt>(mut reader: R) -> io::Result<Self> {
            let command = reader.read_u8()?;
            if command != Self::START_BYTE {
                let err_msg = "invalid \"ping\" command byte";
                return Err(io::Error::new(io::ErrorKind::InvalidData, err_msg));
            }
            Ok(Ping)
        }
    }
}
