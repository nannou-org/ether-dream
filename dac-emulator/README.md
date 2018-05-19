# ether-dream-dac-emulator [![Crates.io](https://img.shields.io/crates/v/ether-dream-dac-emulator.svg)](https://crates.io/crates/ether-dream) [![Crates.io](https://img.shields.io/crates/l/ether-dream-dac-emulator.svg)](https://github.com/nannou-org/ether-dream/blob/master/LICENSE-MIT) [![docs.rs](https://docs.rs/ether-dream-dac-emulator/badge.svg)](https://docs.rs/ether-dream-dac-emulator/)

This contains an **ether-dream-dac-emulator** crate. This crate may be used to
build and run custom, virtual Ether Dream DACs which may be useful for testing
and visualisation.

Emulation includes all networking (UDP broadcasting, TCP listening, TCP
streaming) and the full set of state machines within the DAC (light engine,
playback and source).

Seeing as the virtual DAC does not have a means of emitting physical light, it
instead yields "frame"s of points via an **Output** queue. The rate at which
frames are pushed to this queue may be controlled by specifying the `frame_rate`
field within the DAC **Description**.

The `./dac-emulator/examples` demonstrate how the crate may be used for both
simple command-line testing and for full visualisation of the DAC emulator
output.

```rust
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
```

## Acknowledgements

This crate is based upon the work of Jacob Potter, the creator of the Ether
Dream DAC. Many of the documentation comments are transcribed directly from
their writings in the protocol, while many others are adaptations based on my
understanding of the original writings and the source code itself.

- [ether-dream.com](https://www.ether-dream.com/) - The Ether Dream site.
- [ether-dream.com/protocol](https://www.ether-dream.com/protocol.html) - The
  protocol reference.
- [github.com/j4cbo/j4cDAC](https://github.com/j4cbo/j4cDAC) - The original C
  implementnation.

## License

Licensed under either of

 * Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.


**Contributions**

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
