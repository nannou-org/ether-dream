# ether-dream [![Build Status](https://travis-ci.org/nannou-org/ether-dream.svg?branch=master)](https://travis-ci.org/nannou-org/ether-dream) [![Crates.io](https://img.shields.io/crates/v/ether-dream.svg)](https://crates.io/crates/ether-dream) [![Crates.io](https://img.shields.io/crates/l/ether-dream.svg)](https://github.com/nannou-org/ether-dream/blob/master/LICENSE-MIT) [![docs.rs](https://docs.rs/ether-dream/badge.svg)](https://docs.rs/ether-dream/)

A pure-rust implementation of the Ether Dream Laser DAC protocol.

## Features

This implementation provides:

- **A full implementation of the Ether Dream DAC protocol.**

  Find all types described within the Ether Dream protocol in the **protocol**
  module. This module also provides traits to simplify the process of writing
  and reading protocol types to and from little-endian bytes respectively. This
  is particularly useful for working with **TcpStream**s - the primary method of
  communication between a DAC and the user.

- **An iterator yielding all DAC broadcasts appearing on the network.**

  See the **recv_dac_broadcasts.rs** example for a simple overview. This is the
  first step in locating DACs on the network before attempting to establish
  connections to them.

  ```rust
  for dac_broadcast in ether_dream::recv_dac_broadcasts().unwrap() {
      println!("{:#?}", dac_broadcast);
  }
  ```

- **A simple, low-level and thorough Stream API.**

  This is a thin layer around the **TCP** stream communication channel described
  within the Ether Dream protocol. It simplifies the process of queueing and
  sending **Command**s to the DAC and receiving **Response**s.

  Here's how one might connect to the first DAC they find:

  ```rust
  let (dac_broadcast, source_addr) = ether_dream::recv_dac_broadcasts()
      .unwrap()
      .filter_map(Result::ok)
      .next()
      .unwrap();
  println!("Discovered DAC \"{}\" at \"{}\"! Connecting...", mac_address, source_addr);
  let stream = dac::stream::connect(&dac_broadcast, source_addr.ip().clone()).unwrap();
  ```

  Here's an example of queueing multiple commands for the DAC at once. The
  following code sends the "prepare stream" command, submits some initial point
  data to the DAC and then tells the DAC to begin processing point data at the
  given rate.

  ```rust
  stream
      .queue_commands()
      .prepare_stream()
      .data(generate_points())
      .begin(0, point_hz)
      .submit()
      .unwrap();
  ```

  The **submit** method sends all queued commands over the TCP stream and then
  waits to validate the response for each one. A result is returned along with
  any errors (including NAKs and `io::Error`s) that might have occurred.

  See the **dac_stream.rs** example for a more thorough demonstration of the
  stream API.

  *Note that this API provides no high-level wrapper around the concept of
  "frame"s of "point"s. This is considered a higher-level implementation detail
  and is expected to be implemented by downstream crates. E.g. the nannou
  creative coding framework provides (or will provide) a higher-level laser API
  which uses this crate as one of multiple possible laser protocol targets.*

- **A rust-esque representation of the DAC protocol types.** The **dac** module
  contains several types that mirror the DAC protocol types but with a
  rust-friendly API.

  E.g. Many of the bit-field sets that describe sets of DAC properties have
  `bitflags!` struct representations in this module. Similarly, some state
  machines described by integers in the protocol have rust enum representations
  in this module.

  Methods are provided for converting the protocol representation to this
  rust-esque representation (`from_protocol`) and vice-versa (`to_protocol`).

- **Conventional, clear error handling for all areas of the API.**

## DAC Emulator

[![Crates.io](https://img.shields.io/crates/v/ether-dream-dac-emulator.svg)](https://crates.io/crates/ether-dream) [![Crates.io](https://img.shields.io/crates/l/ether-dream-dac-emulator.svg)](https://github.com/nannou-org/ether-dream/blob/master/LICENSE-MIT) [![docs.rs](https://docs.rs/ether-dream-dac-emulator/badge.svg)](https://docs.rs/ether-dream-dac-emulator/)

This repository also contains an **ether-dream-dac-emulator** crate. This crate
may be used to build and run custom, virtual Ether Dream DACs which may be
useful for testing and visualisation.

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
