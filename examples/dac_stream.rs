extern crate ether_dream;

use ether_dream::dac;
use std::{time, thread};

fn main() {
    println!("Listening for an Ether Dream DAC...");

    let (dac_broadcast, source_addr) = ether_dream::recv_dac_broadcasts()
        .expect("failed to bind to UDP socket")
        .filter_map(Result::ok)
        .next()
        .unwrap();
    let mac_address = dac::MacAddress(dac_broadcast.mac_address);

    println!("Discovered DAC \"{}\" at \"{}\"! Connecting...", mac_address, source_addr);

    // Establish the TCP connection.
    let mut stream = dac::stream::connect(&dac_broadcast, source_addr.ip().clone()).unwrap();

    // If we want to create an animation (in our case a moving sine wave) we need a frame rate.
    let frame_hz = 60.0;
    // Lets use the DAC at half the maximum scan rate.
    let point_hz = stream.dac().max_point_rate / 8;
    // Determine the number of points per frame given our target frame and point rates.
    let points_per_frame = (point_hz as f32 / frame_hz) as u16;
    // Work out the loop interval in milliseconds.
    let loop_interval_ms = time::Duration::from_millis((1_000.0 / frame_hz) as _);
    // Track the time that we started generating points to use as the phase for a sine wave.
    let start = time::Instant::now();

    println!("Preparing for playback:\n\tframe_hz: {}\n\tpoint_hz: {}\n\tpoints_per_frame: {}\n...",
             frame_hz, point_hz, points_per_frame);

    // Prepare the DAC's playback engine and await the repsonse.
    stream
        .queue_commands()
        .prepare_stream()
        .submit()
        .unwrap();

    println!("Beginning playback!");

    // Queue the initial frame and tell the DAC to begin producing output.
    stream
        .queue_commands()
        .data(sine_wave_frame_dac_points(start, points_per_frame))
        .begin(0, point_hz)
        .submit()
        .unwrap();

    // Loop and continue to send points forever.
    loop {
        stream
            .queue_commands()
            .data(sine_wave_frame_dac_points(start, points_per_frame))
            .submit()
            .unwrap();
        thread::sleep(loop_interval_ms);
    }

    // Tell the DAC to stop producing output and return to idle. Wait for the response.
    //
    // Note that the DAC is commanded to stop on `Drop` if this is not called and any errors
    // produced are ignored.
    stream
        .queue_commands()
        .stop()
        .submit()
        .unwrap();
}

// Create a sine wave of white DAC points spanning the entire x and y ranges of the projector.
//
// The phase of the sine wave is based on the duration since the given `start` instant.
//
// The number of points generated will be equal to the given `points_per_frame`.
fn sine_wave_frame_dac_points(
    start: time::Instant,
    points_per_frame: u16,
) -> Vec<ether_dream::protocol::DacPoint>
{
    let duration = start.elapsed();
    let phase = duration.as_secs() as f32 + duration.subsec_nanos() as f32 * 1e-9;

    // Get the amplitude of the sine wav given some fraction across the x axis.
    let amp = |x_fract: f32| ((x_fract + phase) * 2.0 * std::f32::consts::PI).sin();

    // Create a point for the given index within `points_per_frame` with the given rgb colour.
    let point_at_i = |i, r, g, b| -> ether_dream::protocol::DacPoint {
        let i_fract = i as f32 / points_per_frame as f32;
        let x = (std::i16::MIN as f32 + i_fract * (std::i16::MAX as f32 - std::i16::MIN as f32)) as i16;
        //let x = (i as i32 + std::i16::MIN as i32) as i16;
        let y = (amp(i_fract) * std::i16::MAX as f32) as i16;
        ether_dream::protocol::DacPoint {
            control: 0,
            x,
            y,
            i,
            r,
            g,
            b,
            u1: 0,
            u2: 0,
        }
    };

    // A vec for collecting points.
    //
    // Note that it will normally be more efficient to re-use a buffer or to use an iterator,
    // however this example produces a new `Vec` each frame for simplicity.
    let mut dac_points = Vec::with_capacity(points_per_frame as _);

    // Make the first point invisible so that the laser does not create a line from where it
    // previously ended.
    let (i, r, g, b) = (0, 0, 0, 0);
    let dac_point = point_at_i(i, r, g, b);
    dac_points.push(dac_point);

    // Draw the rest of the points white to create a white sine wave.
    for i in 1..points_per_frame {
        let channel_max = std::u16::MAX;
        let (r, g, b) = (channel_max, channel_max, channel_max);
        let dac_point = point_at_i(i, r, g, b);
        dac_points.push(dac_point);
    }

    dac_points
}
