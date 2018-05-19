extern crate ether_dream_dac_emulator;

fn main() {
    let dac_description = Default::default();
    println!("Creating an emulator for the following Ether Dream DAC:\n{:#?}", dac_description);
    let (broadcaster, mut listener) = ether_dream_dac_emulator::new(dac_description).unwrap();
    println!("Broadcasting DAC once per second...");
    let broadcaster_handle = broadcaster.spawn().unwrap();
    broadcaster_handle.spawn_once_per_second_timer().unwrap();
    println!("Listening for stream connection requests...");
    while let Ok((stream, addr)) = listener.accept() {
        println!("Connected to {}!", addr);
        let output = stream.output();
        loop {
            match output.next_frame() {
                Ok(frame) => println!("\tReceived frame with {} points", frame.len()),
                Err(_) => break,
            }
        }
        println!("Stream connection shutdown. Awaiting new requests...");
    }
}
