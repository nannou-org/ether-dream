use futures::prelude::*;

fn main() -> std::io::Result<()> {
    let dac_description = Default::default();
    println!(
        "Creating an emulator for the following Ether Dream DAC:\n{:#?}",
        dac_description
    );

    let (mut broadcaster, mut listener) = ether_dream_dac_emulator::new(dac_description)?;
    let msgs = ether_dream_dac_emulator::broadcaster::one_hz_send();
    let broadcasting = broadcaster.run(msgs.boxed());
    let listening = async move {
        let err = loop {
            let (stream, addr) = match listener.accept().await {
                Err(err) => break err,
                Ok(stream) => stream,
            };
            println!("Connected to {}!", addr);
            let mut i = 0;
            let stream_start = std::time::Instant::now();
            while let Ok(_point_result) = stream.next_points().await {
                i += 1;
                if i % 10000 == 0 {
                    println!("{:?}: Point count: {}", stream_start.elapsed(), i);
                }
            }
            println!("Disconnected from {}.", addr);
        };
        println!("Stopped listening: {}", err);
        Err(err)
    };

    let dac = async move {
        let (b_res, l_res) = future::join(broadcasting, listening).await;
        b_res?;
        l_res?;
        Ok(())
    };

    smol::run(dac)
}
