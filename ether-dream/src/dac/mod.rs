//! A simple abstraction around a single Ether Dream DAC.

pub mod stream;

use byteorder;
use crate::protocol::{self, command, Command as CommandTrait, ReadFromBytes, WriteToBytes};
use std::error::Error;
use std::{fmt, io, ops};
pub use self::stream::Stream;

/// A DAC along with its broadcasted MAC address.
///
/// This type can be used as though it is a `Dac` in many places as it implements
/// `Deref<Target = Dac>`
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Addressed {
    /// The unique MAC address associated with the DAC.
    ///
    /// This may be used to distinguish between multiple DACs broadcasting on a network.
    pub mac_address: MacAddress,
    /// The state of the DAC itself.
    pub dac: Dac,
}

/// A simple abstraction around a single Ether Dream DAC.
///
/// This type monitors the multiple state machines described within the protocol and provides
/// access to information about the Ether Dream hardware and software implementations.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Dac {
    /// This is undocumented in the official protocol but seems to represent a version number for
    /// the hardware in use by the DAC.
    pub hw_revision: u16,
    /// This is undocumented in the official protocol but seems to represent the version of the
    /// protocol implementation. As of writing this, this is hardcoded as `2` in the original
    /// source.
    pub sw_revision: u16,
    /// The maximum number of points that the DAC may buffer at once.
    pub buffer_capacity: u16,
    /// The maximum rate at which the DAC may process buffered points.
    pub max_point_rate: u32,
    /// A more rust-esque version of the `ether_dream::protocol::DacState`.
    ///
    /// The DAC sends its status with each `DacBroadcast` and `DacResponse`.
    pub status: Status,
}

/// The fixed-size array used to represent the MAC address of a DAC.
///
/// This may be used to distinguish between multiple DACs broadcasting on a network.
///
/// This type implements `std::fmt::Display` which can be used to produce the commonly used
/// "human-readable" hex-value string representation of the address (e.g. "2A:FE:54:67:8B:D4").
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub struct MacAddress(pub [u8; 6]);

/// A more rust-esque version of the `ether_dream::protocol::DacState`.
///
/// The DAC sends its status with each `DacBroadcast` and `DacResponse`.
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub struct Status {
    /// This remains undocumented in the protocol.
    ///
    /// The original implementation source simply sets this to `0`.
    pub protocol: u8,
    /// The current state of the DAC's "light engine" state machine.
    pub light_engine: LightEngine,
    /// The current state of the DAC's "playback" state machine.
    pub playback: Playback,
    /// The currently-selected data source.
    pub data_source: DataSource,
    /// If the light engine is `Ready` no flags will be set.
    pub light_engine_flags: LightEngineFlags,
    /// These flags may be non-zero during normal operation.
    pub playback_flags: PlaybackFlags,
    /// The number of points currently buffered within the DAC.
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

/// The light engine state machine - the first of the three primary state machines described within
/// the protocol.
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub enum LightEngine {
    Ready,
    /// In the case where the DAC is also used for thermal control of laser apparatus, this is the
    /// state that is entered after power-up.
    Warmup,
    /// Lasers are off but thermal control is still active.
    Cooldown,
    /// An emergency stop has been triggered, either by an E-stop input on the DAC, an E-stop
    /// command over the network, or a fault such as over-temperature.
    EmergencyStop,
}

/// The DAC has one playback system, which buffers data and sends it to the analog output hardware
/// at its current point rate. At any given time, the playback system is connected to a source.
/// Usually the source is the network streamer, which uses this protocol. However, other sources
/// exist, such as a built-in abstract generator and file playback from SD card. The playback
/// system is in one of the following states.
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub enum Playback {
    /// The default state:
    ///
    /// - No points may be added to the buffer.
    /// - No output is generated.
    /// - All analog outputs are at 0v.
    /// - The shutter is controlled by the data source.
    Idle,
    /// The buffer will accept points.
    ///
    /// The output is the same as the `Idle` state
    Prepared,
    /// Points are being sent to the output.
    Playing,
}

/// The data source in use by a DAC.
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub enum DataSource {
    /// Network streaming (the protocol implemented in this library).
    NetworkStreaming,
    /// ILDA playback from an SD card.
    IldaPlayback(IldaPlaybackFlags),
    /// The DAC's internal abstract generator.
    InternalAbstractGenerator(InternalAbstractGeneratorFlags),
}

bitflags! {
    /// If the light engine is `Ready`, this will be `0`.
    pub struct LightEngineFlags: u16 {
        const EMERGENCY_STOP_PACKET_OR_INVALID_COMMAND = 0b00000001;
        const EMERGENCY_STOP_PROJECTOR_INPUT = 0b00000010;
        const EMERGENCY_STOP_PROJECTOR_INPUT_ACTIVE = 0b00000100;
        const EMERGENCY_STOP_OVER_TEMPERATURE = 0b00001000;
        const EMERGENCY_STOP_OVER_TEMPERATURE_ACTIVE = 0b00010000;
        const EMERGENCY_STOP_LOST_ETHERNET_LINK = 0b00100000;
    }
}

bitflags! {
    /// If the light engine is `Ready`, this will be `0`.
    pub struct PlaybackFlags: u16 {
        /// Describes the state of the shutter.
        const SHUTTER_OPEN = 0b00000001;
        /// Set if the last stream ended with underflow rather than a `Stop`.
        const UNDERFLOWED = 0b00000010;
        /// Set if the last stream ended because the E-Stop state was entered.
        ///
        /// This is reset to zero by the `Prepare` command.
        const EMERGENCY_STOP = 0b00000100;
    }
}

bitflags! {
    /// If the data source is ILDA playback via SD, the following flags are used.
    pub struct IldaPlaybackFlags: u16 {
        const PLAYING = 0b0;
        const REPEAT = 0b1;
    }
}

bitflags! {
    /// If the data source is the internal abstract generator, the following flags are used.
    pub struct InternalAbstractGeneratorFlags: u16 {
        const PLAYING = 0;
    }
}

bitflags! {
    /// The set of flags used to represent the **control** field of a **DacPoint**.
    pub struct PointControl: u16 {
        /// Indicates to the DAC to read a new point rate out of the point rate buffer and set it
        /// as the current playback rate.
        ///
        /// If the buffer is empty, the rate is not changed.
        const CHANGE_RATE = 0b10000000_00000000;
    }
}

/// A command whose kind is determined at runtime.
///
/// This is particularly useful when **Read**ing a command from bytes or from a TCP stream. The
/// **ReadFromBytes** implementation will automatically determine the kind by reading the first
/// byte.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Command<'a> {
    PrepareStream(command::PrepareStream),
    Begin(command::Begin),
    PointRate(command::PointRate),
    Data(command::Data<'a>),
    Stop(command::Stop),
    EmergencyStop(command::EmergencyStop),
    ClearEmergencyStop(command::ClearEmergencyStop),
    Ping(command::Ping),
}

/// An error describing a failure to convert a `protocol::DacStatus` to a `Status`.
#[derive(Debug)]
pub enum ProtocolError {
    UnknownLightEngineState,
    UnknownPlaybackState,
    UnknownDataSource,
}

impl Addressed {
    /// Create an `Addressed` DAC from a received `DacBroadcast`.
    pub fn from_broadcast(dac_broadcast: &protocol::DacBroadcast) -> Result<Self, ProtocolError> {
        let protocol::DacBroadcast {
            mac_address,
            hw_revision,
            sw_revision,
            buffer_capacity,
            max_point_rate,
            dac_status,
        } = *dac_broadcast;
        let mac_address = MacAddress(mac_address);
        let status = Status::from_protocol(&dac_status)?;
        let dac = Dac {
            hw_revision,
            sw_revision,
            buffer_capacity,
            max_point_rate,
            status,
        };
        let addressed = Addressed { mac_address, dac };
        Ok(addressed)
    }
}

impl Dac {
    /// Update the inner status given a new protocol representation.
    pub fn update_status(&mut self, status: &protocol::DacStatus) -> Result<(), ProtocolError> {
        self.status.update(status)
    }
}

impl Status {
    /// Create a `Status` from the lower-level protocol representation.
    pub fn from_protocol(status: &protocol::DacStatus) -> Result<Self, ProtocolError> {
        let protocol = status.protocol;
        let light_engine = LightEngine::from_protocol(status.light_engine_state)
            .ok_or(ProtocolError::UnknownLightEngineState)?;
        let playback = Playback::from_protocol(status.playback_state)
            .ok_or(ProtocolError::UnknownPlaybackState)?;
        let data_source = DataSource::from_protocol(status.source, status.source_flags)
            .ok_or(ProtocolError::UnknownDataSource)?;
        let light_engine_flags = LightEngineFlags::from_bits_truncate(status.light_engine_flags);
        let playback_flags = PlaybackFlags::from_bits_truncate(status.playback_flags);
        let buffer_fullness = status.buffer_fullness;
        let point_rate = status.point_rate;
        let point_count = status.point_count;
        Ok(Status {
            protocol,
            light_engine,
            playback,
            data_source,
            light_engine_flags,
            playback_flags,
            buffer_fullness,
            point_rate,
            point_count,
        })
    }

    /// Update the `Status` from the lower-level protocol representation.
    pub fn update(&mut self, status: &protocol::DacStatus) -> Result<(), ProtocolError> {
        self.protocol = status.protocol;
        self.light_engine = LightEngine::from_protocol(status.light_engine_state)
            .ok_or(ProtocolError::UnknownLightEngineState)?;
        self.playback = Playback::from_protocol(status.playback_state)
            .ok_or(ProtocolError::UnknownPlaybackState)?;
        self.data_source = DataSource::from_protocol(status.source, status.source_flags)
            .ok_or(ProtocolError::UnknownDataSource)?;
        self.light_engine_flags = LightEngineFlags::from_bits_truncate(status.light_engine_flags);
        self.playback_flags = PlaybackFlags::from_bits_truncate(status.playback_flags);
        self.buffer_fullness = status.buffer_fullness;
        self.point_rate = status.point_rate;
        self.point_count = status.point_count;
        Ok(())
    }

    /// Convert the `Status` to its lower-level protocol representation.
    pub fn to_protocol(&self) -> protocol::DacStatus {
        let protocol = self.protocol;
        let light_engine_state = self.light_engine.to_protocol();
        let playback_state = self.playback.to_protocol();
        let (source, source_flags) = self.data_source.to_protocol();
        let light_engine_flags = self.light_engine_flags.bits();
        let playback_flags = self.playback_flags.bits();
        let buffer_fullness = self.buffer_fullness;
        let point_rate = self.point_rate;
        let point_count = self.point_count;
        protocol::DacStatus {
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
        }
    }
}

impl LightEngine {
    /// Create a `LightEngine` enum from the lower-level protocol representation.
    ///
    /// Returns `None` if the given `state` byte is not known.
    pub fn from_protocol(state: u8) -> Option<Self> {
        let light_engine = match state {
            protocol::DacStatus::LIGHT_ENGINE_READY => LightEngine::Ready,
            protocol::DacStatus::LIGHT_ENGINE_WARMUP => LightEngine::Warmup,
            protocol::DacStatus::LIGHT_ENGINE_COOLDOWN => LightEngine::Cooldown,
            protocol::DacStatus::LIGHT_ENGINE_EMERGENCY_STOP => LightEngine::EmergencyStop,
            _ => return None,
        };
        Some(light_engine)
    }

    /// Convert the `LightEngine` enum to its lower-level protocol representation.
    pub fn to_protocol(&self) -> u8 {
        match *self {
            LightEngine::Ready => protocol::DacStatus::LIGHT_ENGINE_READY,
            LightEngine::Warmup => protocol::DacStatus::LIGHT_ENGINE_WARMUP,
            LightEngine::Cooldown => protocol::DacStatus::LIGHT_ENGINE_COOLDOWN,
            LightEngine::EmergencyStop => protocol::DacStatus::LIGHT_ENGINE_EMERGENCY_STOP,
        }
    }
}

impl Playback {
    /// Create a `Playback` enum from the lower-level protocol representation.
    ///
    /// Returns `None` if the given `state` byte is not known.
    pub fn from_protocol(state: u8) -> Option<Self> {
        let playback = match state {
            protocol::DacStatus::PLAYBACK_IDLE => Playback::Idle,
            protocol::DacStatus::PLAYBACK_PREPARED => Playback::Prepared,
            protocol::DacStatus::PLAYBACK_PLAYING => Playback::Playing,
            _ => return None,
        };
        Some(playback)
    }

    /// Convert the `LightEngine` enum to its lower-level protocol representation.
    pub fn to_protocol(&self) -> u8 {
        match *self {
            Playback::Idle => protocol::DacStatus::PLAYBACK_IDLE,
            Playback::Prepared => protocol::DacStatus::PLAYBACK_PREPARED,
            Playback::Playing => protocol::DacStatus::PLAYBACK_PLAYING,
        }
    }
}

impl DataSource {
    /// Create a `DataSource` enum from the lower-level protocol representation.
    ///
    /// Returns `None` if the given `source` byte is not known.
    pub fn from_protocol(source: u8, flags: u16) -> Option<Self> {
        match source {
            protocol::DacStatus::SOURCE_NETWORK_STREAMING => {
                Some(DataSource::NetworkStreaming)
            },
            protocol::DacStatus::SOURCE_ILDA_PLAYBACK_SD => {
                Some(IldaPlaybackFlags::from_bits_truncate(flags))
                    .map(DataSource::IldaPlayback)
            },
            protocol::DacStatus::SOURCE_INTERNAL_ABSTRACT_GENERATOR => {
                Some(InternalAbstractGeneratorFlags::from_bits_truncate(flags))
                    .map(DataSource::InternalAbstractGenerator)
            },
            _ => None,
        }
    }

    /// Convert the `LightEngine` enum to its lower-level protocol representation.
    ///
    /// Returns the `source` and the `source_flags` fields respectively.
    pub fn to_protocol(&self) -> (u8, u16) {
        match *self {
            DataSource::NetworkStreaming => {
                (protocol::DacStatus::SOURCE_NETWORK_STREAMING, 0)
            },
            DataSource::IldaPlayback(ref flags) => {
                (protocol::DacStatus::SOURCE_ILDA_PLAYBACK_SD, flags.bits())
            },
            DataSource::InternalAbstractGenerator(ref flags) => {
                (protocol::DacStatus::SOURCE_INTERNAL_ABSTRACT_GENERATOR, flags.bits())
            },
        }
    }
}

impl ops::Deref for Addressed {
    type Target = Dac;
    fn deref(&self) -> &Self::Target {
        &self.dac
    }
}

impl ops::DerefMut for Addressed {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.dac
    }
}

impl From<[u8; 6]> for MacAddress {
    fn from(bytes: [u8; 6]) -> Self {
        MacAddress(bytes)
    }
}

impl Into<[u8; 6]> for MacAddress {
    fn into(self) -> [u8; 6] {
        self.0
    }
}

impl ops::Deref for MacAddress {
    type Target = [u8; 6];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl fmt::Display for MacAddress {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let a = &self.0;
        write!(f, "{:X}:{:X}:{:X}:{:X}:{:X}:{:X}", a[0], a[1], a[2], a[3], a[4], a[5])
    }
}

impl ReadFromBytes for Command<'static> {
    fn read_from_bytes<R: byteorder::ReadBytesExt>(mut reader: R) -> io::Result<Self> {
        let command = reader.read_u8()?;
        let kind = match command {
            command::PrepareStream::START_BYTE => command::PrepareStream.into(),
            command::Begin::START_BYTE => command::Begin::read_fields(reader)?.into(),
            command::PointRate::START_BYTE => command::PointRate::read_fields(reader)?.into(),
            command::Data::START_BYTE => command::Data::read_fields(reader)?.into(),
            command::Stop::START_BYTE => command::Stop.into(),
            command::EmergencyStop::START_BYTE => command::EmergencyStop.into(),
            command::EmergencyStopAlt::START_BYTE => command::EmergencyStop.into(),
            command::ClearEmergencyStop::START_BYTE => command::ClearEmergencyStop.into(),
            command::Ping::START_BYTE => command::Ping.into(),
            unknown => {
                let err_msg = format!("invalid command byte \"{}\"", unknown);
                return Err(io::Error::new(io::ErrorKind::InvalidData, err_msg));
            }
        };
        Ok(kind)
    }
}

impl<'a> WriteToBytes for Command<'a> {
    fn write_to_bytes<W: byteorder::WriteBytesExt>(&self, writer: W) -> io::Result<()> {
        match *self {
            Command::PrepareStream(ref cmd) => cmd.write_to_bytes(writer),
            Command::Begin(ref cmd) => cmd.write_to_bytes(writer),
            Command::PointRate(ref cmd) => cmd.write_to_bytes(writer),
            Command::Data(ref cmd) => cmd.write_to_bytes(writer),
            Command::Stop(ref cmd) => cmd.write_to_bytes(writer),
            Command::EmergencyStop(ref cmd) => cmd.write_to_bytes(writer),
            Command::ClearEmergencyStop(ref cmd) => cmd.write_to_bytes(writer),
            Command::Ping(ref cmd) => cmd.write_to_bytes(writer),
        }
    }
}

impl<'a> From<command::PrepareStream> for Command<'a> {
    fn from(command: command::PrepareStream) -> Self {
        Command::PrepareStream(command)
    }
}

impl<'a> From<command::Begin> for Command<'a> {
    fn from(command: command::Begin) -> Self {
        Command::Begin(command)
    }
}

impl<'a> From<command::PointRate> for Command<'a> {
    fn from(command: command::PointRate) -> Self {
        Command::PointRate(command)
    }
}

impl<'a> From<command::Data<'a>> for Command<'a> {
    fn from(command: command::Data<'a>) -> Self {
        Command::Data(command)
    }
}

impl<'a> From<command::Stop> for Command<'a> {
    fn from(command: command::Stop) -> Self {
        Command::Stop(command)
    }
}

impl<'a> From<command::EmergencyStop> for Command<'a> {
    fn from(command: command::EmergencyStop) -> Self {
        Command::EmergencyStop(command)
    }
}

impl<'a> From<command::EmergencyStopAlt> for Command<'a> {
    fn from(_: command::EmergencyStopAlt) -> Self {
        Command::EmergencyStop(command::EmergencyStop)
    }
}

impl<'a> From<command::ClearEmergencyStop> for Command<'a> {
    fn from(command: command::ClearEmergencyStop) -> Self {
        Command::ClearEmergencyStop(command)
    }
}

impl<'a> From<command::Ping> for Command<'a> {
    fn from(command: command::Ping) -> Self {
        Command::Ping(command)
    }
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let s = match *self {
            ProtocolError::UnknownLightEngineState => "unknown light engine state",
            ProtocolError::UnknownPlaybackState => "unknown playback state",
            ProtocolError::UnknownDataSource => "unknown data source",
        };
        write!(f, "{}", s)
    }
}

impl Error for ProtocolError {}
