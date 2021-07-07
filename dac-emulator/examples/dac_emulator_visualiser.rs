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

use ether_dream_dac_emulator::ether_dream::protocol::DacPoint;
use ether_dream_dac_emulator::Description;
use futures::prelude::*;
use nannou::prelude::*;
use std::sync::mpsc;
use std::time::Duration;

fn main() {
    nannou::app(model).update(update).run();
}

struct Model {
    active_stream: Option<std::net::SocketAddr>,
    perceived_points: Vec<(std::time::Instant, DacPoint)>,
    event_rx: mpsc::Receiver<Event>,
    underflowing: bool,
}

#[derive(Debug)]
enum Event {
    /// The DAC connected to the given address.
    Connected(std::net::SocketAddr),
    /// Some points were emitted by the DAC.
    Points(std::time::Instant, Vec<DacPoint>),
    /// The DAC's point buffer underflowed.
    Underflowed,
    /// The DAC disconnected from the stream.
    Disconnected,
}

fn model(app: &App) -> Model {
    app.new_window().view(view).build().unwrap();

    // A descriptor of the DAC. Change this to change behaviour, MAC address, etc.
    let dac_description = Default::default();

    // Kick off the `smol` runtime with 4 threads.
    for _ in 0..4 {
        std::thread::spawn(|| smol::run(future::pending::<()>()));
    }

    // Run the DAC emulator on its own thread.
    let (event_tx, event_rx) = mpsc::sync_channel(1_024);
    std::thread::spawn(move || smol::block_on(run_dac(dac_description, event_tx)));

    let active_stream = None;
    let perceived_points = Vec::new();
    let underflowing = false;

    Model {
        active_stream,
        event_rx,
        perceived_points,
        underflowing,
    }
}

fn update(_app: &App, model: &mut Model, _update: Update) {
    // Process all events.
    for event in model.event_rx.try_iter() {
        match event {
            Event::Underflowed => model.underflowing = true,
            Event::Connected(addr) => model.active_stream = Some(addr),
            Event::Disconnected => model.active_stream = None,
            Event::Points(when, pts) => {
                model.underflowing = false;
                let new_pts = pts.into_iter().map(|pt| (when, pt));
                model.perceived_points.extend(new_pts);
            }
        }
    }

    // Drain the old points.
    let now = std::time::Instant::now();
    let too_old = model
        .perceived_points
        .iter()
        .position(|&(t, _)| match t < now {
            true if now.duration_since(t) > PERSISTENCE_OF_VISION => false,
            _ => true,
        })
        .unwrap_or(model.perceived_points.len());
    model.perceived_points.drain(0..too_old);
}

// Draw the state of your `Model` into the given `Frame` here.
fn view(app: &App, model: &Model, frame: Frame) {
    // Begin drawing
    let draw = app.draw();
    let win = app.window_rect();

    // Clear the background to blue.
    draw.background().color(BLACK);

    // Draw the status in the bottom left.
    let (status_string, color) = match model.active_stream {
        None => {
            let s = format!("Awaiting connection...");
            let color = WHITE;
            (s, color)
        }
        Some(addr) => {
            let (buffer, color) = match model.underflowing {
                false => (format!(""), GREEN),
                true => (format!(" (underflowing!)"), RED),
            };
            let s = format!("Connected{}: {}", buffer, addr);
            (s, color)
        }
    };
    let text_area = win.pad(20.0);
    draw.text(&status_string)
        .wh(text_area.wh())
        .left_justify()
        .align_text_bottom()
        .font_size(18)
        .color(color);

    // Draw the path.
    let t_x = |xi: i16| (xi as f32 / std::i16::MAX as f32) * win.w() * 0.5;
    let t_y = |yi: i16| (yi as f32 / std::i16::MAX as f32) * win.h() * 0.5;
    let t_color = |color: u16| color as f32 / std::u16::MAX as f32;
    let convert_point = |pt: &DacPoint, lum: f32| -> (Point2, LinSrgb) {
        let x = t_x(pt.x);
        let y = t_y(pt.y);
        let r = t_color(pt.r) * lum;
        let g = t_color(pt.g) * lum;
        let b = t_color(pt.b) * lum;
        (pt2(x, y), lin_srgb(r, g, b))
    };

    let color_blend = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::SrcAlpha,
        //dst_factor: BlendFactor::OneMinusSrcAlpha,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    };
    let alpha_blend = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    };
    let draw2 = draw.color_blend(color_blend).alpha_blend(alpha_blend);

    //let draw2 = draw.color_blend(BLEND_ADD);
    let now = std::time::Instant::now();
    let mut last_pt: Option<(Point2, LinSrgb)> = None;
    for &(inst, pt) in model.perceived_points.iter() {
        let luminance = match inst < now {
            true => persistence_of_vision(now.duration_since(inst)),
            false => 1.0,
        };
        let a = convert_point(&pt, luminance * 0.25);
        if let Some(b) = last_pt {
            if !is_black(&a.1) || !is_black(&b.1) {
                let weight = 5.0 * luminance;
                if a.0 == b.0 {
                    let rgb = lightest(&a.1, &b.1);
                    draw2.ellipse().xy(a.0).color(rgb).radius(weight * 0.5);
                } else {
                    let pts = [a, b];
                    draw2
                        .polyline()
                        .weight(weight)
                        .caps_round()
                        .points_colored(pts.iter().cloned());
                }
            }
        }
        last_pt = Some(a);
    }

    draw.to_frame(app, &frame).unwrap();
}

async fn run_dac(desc: Description, event_tx: mpsc::SyncSender<Event>) -> std::io::Result<()> {
    let (mut broadcaster, mut listener) = ether_dream_dac_emulator::new(desc)?;
    let msgs = ether_dream_dac_emulator::broadcaster::one_hz_send();
    let broadcasting = broadcaster.run(msgs.boxed());
    let listening = async move {
        loop {
            let (stream, addr) = match listener.accept().await {
                Err(err) => break Err(err),
                Ok(stream) => stream,
            };
            if event_tx.send(Event::Connected(addr)).is_err() {
                break Ok(());
            }
            while let Ok(points_result) = stream.next_points().await {
                let event = match points_result {
                    Err(_) => Event::Underflowed,
                    Ok(points) => Event::Points(std::time::Instant::now(), points),
                };
                event_tx.try_send(event).ok();
            }
            if event_tx.send(Event::Disconnected).is_err() {
                break Ok(());
            }
        }
    };
    let (b_res, l_res) = future::join(broadcasting, listening).await;
    b_res?;
    l_res?;
    Ok(())
}

const PERSISTENCE_OF_VISION: Duration = Duration::from_millis(250);

// Produce a colour that is the lightest channels of the two given colours.
fn lightest(a: &LinSrgb, b: &LinSrgb) -> LinSrgb {
    let r = a.red.max(b.red);
    let g = a.green.max(b.green);
    let b = a.blue.max(b.blue);
    lin_srgb(r, g, b)
}

fn is_black(c: &LinSrgb) -> bool {
    c.red == 0.0 && c.green == 0.0 && c.blue == 0.0
}

// Given some duration since a point was emitted, produce a multiplier for its perceived
// luminosity.
fn persistence_of_vision(duration: Duration) -> f32 {
    let secs = duration.as_secs_f32();
    let max = PERSISTENCE_OF_VISION.as_secs_f32();
    let pow = 4.0; // Found via tweaking.
    (1.0 - (secs / max)).max(0.0).powf(pow)
}
