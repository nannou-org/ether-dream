extern crate ether_dream;

use ether_dream::protocol::{command, DacBroadcast, DacPoint, DacResponse, DacStatus};
use ether_dream::protocol::{Command, ReadBytes, SizeBytes, WriteBytes};
use std::borrow::Cow;

// Writes the given struct to a buffer then reads it.
//
// Returns `true` if the `read` struct is identical to the original.
macro_rules! write_then_read {
    ($struct: expr, $T: ty, $LEN_BYTES: expr) => {{
        let mut bytes = [0u8; $LEN_BYTES];
        {
            let mut writer = &mut bytes[..];
            writer.write_bytes(&$struct).unwrap();
        }
        let mut reader = &bytes[..];
        let result = reader.read_bytes::<$T>().unwrap();
        $struct == result
    }};
}

// A simple `DacStatus` struct shared between tests.
fn dac_status() -> DacStatus {
    DacStatus {
        protocol: 1,
        light_engine_state: DacStatus::LIGHT_ENGINE_READY,
        playback_state: DacStatus::PLAYBACK_IDLE,
        source: DacStatus::SOURCE_NETWORK_STREAMING,
        light_engine_flags: 0,
        playback_flags: 0,
        source_flags: 0,
        buffer_fullness: 0,
        point_rate: 0,
        point_count: 0,
    }
}

// A simple `DacPoint` struct shared between tests.
fn dac_point() -> DacPoint {
    DacPoint {
        control: 0,
        x: -1_024,
        y: 12_345,
        i: 7,
        r: 60_000,
        g: 50_000,
        b: 40_000,
        u1: 1,
        u2: 2,
    }
}

#[test]
fn write_then_read_dac_status() {
    let dac_status = dac_status();
    assert!(write_then_read!(
        dac_status,
        DacStatus,
        DacStatus::SIZE_BYTES
    ));
}

#[test]
fn write_then_read_dac_broadcast() {
    let dac_broadcast = DacBroadcast {
        mac_address: [0x01, 0x23, 0x45, 0x67, 0x89, 0xAB],
        hw_revision: 1,
        sw_revision: 1,
        buffer_capacity: std::u16::MAX,
        max_point_rate: 40_000,
        dac_status: dac_status(),
    };
    assert!(write_then_read!(
        dac_broadcast,
        DacBroadcast,
        DacBroadcast::SIZE_BYTES
    ));
}

#[test]
fn write_then_read_dac_point() {
    let dac_point = dac_point();
    assert!(write_then_read!(dac_point, DacPoint, DacPoint::SIZE_BYTES));
}

#[test]
fn write_then_read_dac_response() {
    let dac_response = DacResponse {
        response: DacResponse::ACK,
        command: command::Begin::START_BYTE,
        dac_status: dac_status(),
    };
    assert!(write_then_read!(
        dac_response,
        DacResponse,
        DacResponse::SIZE_BYTES
    ));
}

#[test]
fn write_then_read_command_prepare_stream() {
    let command = command::PrepareStream;
    assert!(write_then_read!(
        command,
        command::PrepareStream,
        command::PrepareStream::SIZE_BYTES
    ));
}

#[test]
fn write_then_read_command_begin() {
    let low_water_mark = 0;
    let point_rate = 40_000;
    let begin = command::Begin {
        low_water_mark,
        point_rate,
    };
    assert!(write_then_read!(
        begin,
        command::Begin,
        command::Begin::SIZE_BYTES
    ));
}

#[test]
fn write_then_read_command_queue_change() {
    let queue_change = command::PointRate(40_000);
    assert!(write_then_read!(
        queue_change,
        command::PointRate,
        command::PointRate::SIZE_BYTES
    ));
}

#[test]
fn write_then_read_command_data() {
    let n_points = 1024;
    let points = Cow::Owned(vec![dac_point(); n_points]);
    let data = command::Data { points };
    let mut bytes = Vec::with_capacity(n_points * DacPoint::SIZE_BYTES);
    {
        let writer = &mut bytes;
        writer.write_bytes(&data).unwrap();
    }
    let mut reader = &bytes[..];
    let result = reader.read_bytes::<command::Data<'static>>().unwrap();
    assert_eq!(data, result);
}

#[test]
fn write_then_read_command_stop() {
    let command = command::Stop;
    assert!(write_then_read!(
        command,
        command::Stop,
        command::Stop::SIZE_BYTES
    ));
}

#[test]
fn write_then_read_command_emergency_stop() {
    let command = command::EmergencyStop;
    assert!(write_then_read!(
        command,
        command::EmergencyStop,
        command::EmergencyStop::SIZE_BYTES
    ));
}

#[test]
fn write_then_read_command_clear_emergency_stop() {
    let command = command::ClearEmergencyStop;
    assert!(write_then_read!(
        command,
        command::ClearEmergencyStop,
        command::ClearEmergencyStop::SIZE_BYTES
    ));
}

#[test]
fn write_then_read_command_ping() {
    let command = command::Ping;
    assert!(write_then_read!(
        command,
        command::Ping,
        command::Ping::SIZE_BYTES
    ));
}
