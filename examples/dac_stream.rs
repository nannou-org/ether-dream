extern crate ether_dream;

use ether_dream::dac;

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
    let frames_per_second = 60.0;
    // Lets use the DAC at an eighth the maximum scan rate.
    let points_per_second = stream.dac().max_point_rate / 32;
    // Determine the number of points per frame given our target frame and point rates.
    let points_per_frame = (points_per_second as f32 / frames_per_second) as u16;

    println!("Preparing for playback:\n\tframe_hz: {}\n\tpoint_hz: {}\n\tpoints_per_frame: {}\n",
             frames_per_second, points_per_second, points_per_frame);

    // Prepare the DAC's playback engine and await the repsonse.
    stream
        .queue_commands()
        .prepare_stream()
        .submit()
        .unwrap();

    println!("Beginning playback!");

    // The sine wave used to generate points.
    let mut sine_wave = SineWave { point: 0, points_per_frame, frames_per_second };

    // Queue the initial frame and tell the DAC to begin producing output.
    let n_points = points_to_generate(stream.dac());;
    stream
        .queue_commands()
        .data(sine_wave.by_ref().take(n_points))
        .begin(0, points_per_second)
        .submit()
        .unwrap();

    // Loop and continue to send points forever.
    loop {
        // Determine how many points the DAC can currently receive.
        let n_points = points_to_generate(stream.dac());;
        stream
            .queue_commands()
            .data(sine_wave.by_ref().take(n_points))
            .submit()
            .unwrap();
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

// Determine the number of points needed to fill the DAC.
fn points_to_generate(dac: &ether_dream::dac::Dac) -> usize {
    dac.buffer_capacity as usize - 1 - dac.status.buffer_fullness as usize
}

// An iterator that endlessly generates a sine wave of DAC points.
//
// The sine wave oscillates at a rate of once per second.
struct SineWave {
    point: u32,
    points_per_frame: u16,
    frames_per_second: f32,
}

impl Iterator for SineWave {
    type Item = ether_dream::protocol::DacPoint;
    fn next(&mut self) -> Option<Self::Item> {
        let coloured_points_per_frame = self.points_per_frame - 1;
        let i = (self.point % self.points_per_frame as u32) as u16;
        let fract = i as f32 / coloured_points_per_frame as f32;
        let phase = (self.point as f32 / coloured_points_per_frame as f32) / self.frames_per_second;
        let amp = ((fract + phase) * 2.0 * std::f32::consts::PI).sin();
        let (r, g, b) = match i == coloured_points_per_frame {
            true => (0, 0, 0), // Draw the last point black to avoid tracing back to start.
            false => (std::u16::MAX, std::u16::MAX, std::u16::MAX),
        };
        let x_min = std::i16::MIN;
        let x_max = std::i16::MAX;
        let x = (x_min as f32 + fract * (x_max as f32 - x_min as f32)) as i16;
        let y = (amp * x_max as f32) as i16;
        let control = 0;
        let (u1, u2) = (0, 0);
        let p = ether_dream::protocol::DacPoint { control, x, y, i, r, g, b, u1, u2 };
        self.point += 1;
        Some(p)
    }
}
