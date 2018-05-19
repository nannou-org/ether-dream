//! An Ether Dream DAC emulator.
//!
//! The emulator is composed of two primary processes:
//!
//! 1. The **Broadcaster**, responsible for broadcasting over UDP.
//! 2. The **Listener**, responsible for handling connections via TCP as **Stream**s.
//!
//! The DAC components can be created via the **new** constructor which takes a **Description** of
//! the DAC that you wish to emulate.

extern crate crossbeam;
pub extern crate ether_dream;

use ether_dream::dac::{self, Dac};
use ether_dream::protocol;
use std::{io, net};

pub mod broadcaster;
pub mod listener;
pub mod stream;

pub use broadcaster::Broadcaster;
pub use listener::Listener;

pub mod default {
    /// A fake MAC address for the DAC.
    pub const MAC_ADDRESS: [u8; 6] = [0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC];
    /// The maximum point rate specified within the official Ether Dream DAC.
    pub const MAX_POINT_RATE: u32 = 100_000;
    /// The hardware unique identifier. This is undocumented in the protocol, so we just use `0`.
    pub const HW_REVISION: u16 = 0;
    /// The software unique identifier. This is undocumented in the protocol, so we just use `0`.
    pub const SW_REVISION: u16 = 0;
    /// The buffer capacity specified within the official Ether Dream DAC.
    pub const BUFFER_CAPACITY: u16 = 1800;
    /// The default IP address used for broadcasting.
    pub const BROADCAST_IP: [u8; 4] = [255, 255, 255, 255];
    /// The default port to which the UDP broadcaster will bind.
    pub const BROADCASTER_BIND_PORT: u16 = 9001;
    /// The number of frames per second yielded by the **stream::Output**.
    pub const OUTPUT_FRAME_RATE: u32 = 60;
}

/// A type that allows the user to describe a custom Ether Dream DAC emulator.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Description {
    pub mac_address: dac::MacAddress,
    pub max_point_rate: u32,
    pub hw_revision: u16,
    pub sw_revision: u16,
    pub buffer_capacity: u16,
    /// The IP address used for broadcasting.
    pub broadcast_ip: net::Ipv4Addr,
    /// The network socket address port to which the UDP broadcaster should bind to.
    ///
    /// This is an unimportant implementation detail, however we allow specifying it in case the
    /// default causes conflicts for the user.
    pub broadcaster_bind_port: u16,
    /// The number of frames per second yielded by the **stream::Output**.
    pub output_frame_rate: u32,
}

impl Default for Description {
    fn default() -> Self {
        Description {
            mac_address: default::MAC_ADDRESS.into(),
            max_point_rate: default::MAX_POINT_RATE,
            hw_revision: default::HW_REVISION,
            sw_revision: default::SW_REVISION,
            buffer_capacity: default::BUFFER_CAPACITY,
            broadcast_ip: net::Ipv4Addr::from(default::BROADCAST_IP),
            broadcaster_bind_port: default::BROADCASTER_BIND_PORT,
            output_frame_rate: default::OUTPUT_FRAME_RATE,
        }
    }
}

/// The initial status of the DAC.
pub fn initial_status() -> protocol::DacStatus {
    protocol::DacStatus {
        protocol: 0,
        light_engine_state: protocol::DacStatus::LIGHT_ENGINE_READY,
        playback_state: protocol::DacStatus::PLAYBACK_IDLE,
        source: protocol::DacStatus::SOURCE_NETWORK_STREAMING,
        light_engine_flags: 0,
        playback_flags: 0,
        source_flags: 0,
        buffer_fullness: 0,
        point_rate: 0,
        point_count: 0,
    }
}

/// Create a new, initial DAC emulator with a `status` produced via the `initial_status` function.
pub fn new(description: Description) -> io::Result<(Broadcaster, Listener)> {
    let Description {
        mac_address,
        hw_revision,
        sw_revision,
        buffer_capacity,
        max_point_rate,
        broadcast_ip,
        broadcaster_bind_port,
        output_frame_rate,
    } = description;

    let dac_status = initial_status();
    let status = dac::Status::from_protocol(&dac_status)
        .expect("failed to interpret initial state");
    let dac = Dac {
        hw_revision,
        sw_revision,
        buffer_capacity,
        max_point_rate,
        status,
    };
    let dac = dac::Addressed { mac_address, dac };
    let broadcaster = Broadcaster::new(dac, broadcaster_bind_port, broadcast_ip)?;
    let listener = Listener::new(dac, output_frame_rate)?;
    Ok((broadcaster, listener))
}
