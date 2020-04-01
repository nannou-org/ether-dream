//! A simple nannou app for visualising the output data produced by the Ether Dream DAC emulator.
//!
//! In this example we:
//!
//! 1. Create the default DAC emulator.
//! 2. Spawn the broadcaster on its own thread so that it sends UDP broadcasts once per second.
//! 3. Spawn the listener on its own thread so that it may listen for stream connection requests.
//! 4. Loop at 60 FPS (nannou's default app loop).
//! 5. On each loop, check whether or not a new stream has been established.
//! 6. If we have a stream, check for the latest frame points.
//! 7. In our `view` function, draw the laser frame to the bounds of the window.

extern crate ether_dream_dac_emulator;
extern crate nannou;

use ether_dream_dac_emulator::{ether_dream, broadcaster, listener};
use nannou::prelude::*;
use std::sync::mpsc;
use std::{net, thread};

fn main() {
    nannou::app(model).update(update).run();
}

struct Model {
    _broadcaster: broadcaster::Handle,
    stream: Option<listener::ActiveStream>,
    frame_points: Vec<ether_dream::protocol::DacPoint>,
    stream_rx: mpsc::Receiver<(listener::ActiveStream, net::SocketAddr)>,
}

fn model(app: &App) -> Model {
    app.new_window().view(view).build().unwrap();

    let dac_description = Default::default();
    let (broadcaster, mut listener) = ether_dream_dac_emulator::new(dac_description).unwrap();

    // Run the DAC broadcaster.
    let broadcaster = broadcaster.spawn().unwrap();
    broadcaster.spawn_once_per_second_timer().unwrap();

    // Spawn a thread for the listener.
    let (stream_tx, stream_rx) = mpsc::channel();
    thread::spawn(move || {
        while let Ok((stream, addr)) = listener.accept() {
            if stream_tx.send((stream, addr)).is_err() {
                break;
            }
        }
    });

    // Initialise the stream to `None`.
    let stream = None;

    // The buffer to use for collecting frame points.
    let frame_points = Vec::new();

    Model { _broadcaster: broadcaster, stream, stream_rx, frame_points }
}

fn update(_app: &App, model: &mut Model, _update: Update) {
    // Check for stream connections.
    if let Ok((stream, addr)) = model.stream_rx.try_recv() {
        println!("Connected to {}!", addr);
        model.stream = Some(stream);
    }

    // Check for new frames.
    if let Some(output) = model.stream.as_ref().map(|stream| stream.output()) {
        let mut latest_frame = None;
        loop {
            match output.try_next_frame() {
                Err(_) => {
                    println!("Stream shutdown.");
                    model.stream.take();
                },
                Ok(None) => break,
                Ok(Some(frame)) => {
                    latest_frame = Some(frame);
                }
            }
        }
        if let Some(frame) = latest_frame {
            model.frame_points.clear();
            model.frame_points.extend(frame.iter().cloned());
        }
    }
}

// Draw the state of your `Model` into the given `Frame` here.
fn view(app: &App, model: &Model, frame: Frame) {
    // Begin drawing
    let draw = app.draw();

    // Clear the background to blue.
    draw.background().color(BLACK);

    let win_rect = app.window_rect();

    let t_x = |xi: i16| (xi as f32 / std::i16::MAX as f32) * win_rect.w() * 0.5;
    let t_y = |yi: i16| (yi as f32 / std::i16::MAX as f32) * win_rect.h() * 0.5;
    let t_color = |color: u16| color as f32 / std::u16::MAX as f32;
    let convert_point = |pt: &ether_dream::protocol::DacPoint| -> (Point2<f32>, (f32, f32, f32)) {
        let x = t_x(pt.x);
        let y = t_y(pt.y);
        let r = t_color(pt.r);
        let g = t_color(pt.g);
        let b = t_color(pt.b);
        (pt2(x, y), (r, g, b))
    };

    for window in model.frame_points.windows(2) {
        let (ap, (ar, ag, ab)) = convert_point(&window[0]);
        let (bp, (_br, _bg, _bb)) = convert_point(&window[1]);
        draw.line()
            .points(ap, bp)
            .rgb(ar, ag, ab);
    }

    if model.stream.is_none() {
        draw.text("Awaiting connection...")
            .w(300.0)
            .font_size(24)
            .color(RED);
    }


    draw.to_frame(app, &frame).unwrap();
}
