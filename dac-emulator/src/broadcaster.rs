//! Items related to the DAC emulator's broadcasting process.

use ether_dream::dac;
use ether_dream::protocol::{DacBroadcast, SizeBytes, WriteBytes, BROADCAST_PORT};
use std::sync::atomic::{self, AtomicBool};
use std::sync::{mpsc, Arc, Mutex};
use std::{io, net, thread, time};

/// The broadcasting side of the DAC.
///
/// Once `run` is called, the broadcaster will block and loop at a rate of once-per-second,
/// broadcasting the current state of the DAC on each iteration.
pub struct Broadcaster {
    // The last received copy of the state of the DAC.
    dac: dac::Addressed,
    // The UDP socket used for broadcasting.
    udp_socket: net::UdpSocket,
    // The address to which the broadcaster is sending.
    broadcast_addr: net::SocketAddrV4,
    // A buffer to use for writing `DacBroadcast`s to bytes for the UDP socket.
    bytes: [u8; DacBroadcast::SIZE_BYTES],
}

/// A **Handle** for asynchronously communicating with the `Broadcaster` thread.
///
/// **Handle** is returned by the **Broadcaster::spawn** method.
#[derive(Clone)]
pub struct Handle {
    // For sending messages to the broadcaster.
    tx: Tx,
    // A handle to the broadcaster thread.
    thread: Arc<Mutex<Option<thread::JoinHandle<io::Result<Broadcaster>>>>>,
    // A handle to the
    once_per_second_timer: Arc<Mutex<Option<Timer>>>,
}

/// A handle to a thread that sends **Message::Send** repeatedly at a given interval.
pub struct Timer {
    is_closed: Arc<AtomicBool>,
    _thread: thread::JoinHandle<()>,
}

/// The sending end of the **Broadcaster** **Message** channel.
pub type Tx = mpsc::Sender<Message>;

/// The receiving end of the **Broadcaster** **Message** channel.
pub type Rx = mpsc::Receiver<Message>;

/// **Broadcaster::run** blocks on the following messages.
#[derive(Debug)]
pub enum Message {
    /// The state of the DAC has changed.
    Dac(dac::Addressed),
    /// Tell the broadcaster to send a message.
    Send,
    /// Tell the broadcaster to stop running and break from its loop.
    Close,
}

impl Timer {
    /// Consumer `self` and notifies the **Timer** thread to stop.
    pub fn close(&self) {
        self.is_closed.store(true, atomic::Ordering::Relaxed);
    }
}

impl Handle {
    /// Send a **Message::Send** to the **Broadcaster** thread, causing it to send a
    /// **DacBroadcast** over UDP.
    ///
    /// Returns an **Err** if the **Broadcaster** thread has been closed.
    pub fn send(&self) -> Result<(), mpsc::SendError<()>> {
        self.tx.send(Message::Send).map_err(|_| mpsc::SendError(()))
    }

    /// Send a DAC update to the **Broadcaster** thread.
    ///
    /// Returns an **Err** if the **Broadcaster** thread has been closed.
    pub fn dac(&self, dac: dac::Addressed) -> Result<(), mpsc::SendError<()>> {
        self.tx
            .send(Message::Dac(dac))
            .map_err(|_| mpsc::SendError(()))
    }

    /// Spawns a **Timer** that sends **Message::Send** to the **Broadcaster** thread once per
    /// second.
    ///
    /// If this was already called and a timer is already running, the existing timer will be
    /// closed and this timer will replace it.
    pub fn spawn_once_per_second_timer(&self) -> io::Result<()> {
        let mut guard = self
            .once_per_second_timer
            .lock()
            .expect("failed to lock timer");
        guard.take();
        *guard = Some(spawn_once_per_second_timer(self.tx.clone())?);
        Ok(())
    }

    // Consume `self` and return the handle to the **Broadcaster** thread for `join`ing.
    //
    // `None` is returned if the **close** was already called from another **Handle**.
    fn close_inner(&self) -> Option<thread::JoinHandle<io::Result<Broadcaster>>> {
        self.tx.send(Message::Close).ok();
        self.thread.lock().unwrap().take()
    }

    /// Consume `self` and return the handle to the **Broadcaster** thread for `join`ing.
    ///
    /// `None` is returned if the **close** was already called from another **Handle**.
    pub fn close(self) -> Option<thread::JoinHandle<io::Result<Broadcaster>>> {
        self.close_inner()
    }
}

impl Broadcaster {
    /// Create a new **Broadcaster** initialised with the given DAC state.
    ///
    /// Produces an **io::Error** if creating the UDP socket fails or if enabling broadcast fails.
    pub fn new(
        dac: dac::Addressed,
        bind_port: u16,
        broadcast_ip: net::Ipv4Addr,
    ) -> io::Result<Broadcaster> {
        let broadcast_addr = net::SocketAddrV4::new(broadcast_ip, BROADCAST_PORT);
        let bind_addr = net::SocketAddrV4::new([0, 0, 0, 0].into(), bind_port);
        let udp_socket = net::UdpSocket::bind(bind_addr)?;
        udp_socket.set_broadcast(true)?;
        let bytes = [0u8; DacBroadcast::SIZE_BYTES];
        Ok(Broadcaster {
            dac,
            udp_socket,
            broadcast_addr,
            bytes,
        })
    }

    /// Creates a **DacBroadcast** from the current known DAC state.
    ///
    /// This is used within the **send** method.
    pub fn create_broadcast(&self) -> DacBroadcast {
        // Retrieve the current DAC status.
        let dac_status = self.dac.status.to_protocol();

        // Create the broadcast message.
        DacBroadcast {
            mac_address: self.dac.mac_address.into(),
            hw_revision: self.dac.hw_revision,
            sw_revision: self.dac.sw_revision,
            buffer_capacity: self.dac.buffer_capacity,
            max_point_rate: self.dac.max_point_rate,
            dac_status,
        }
    }

    /// Sends a single broadcast message over the inner UDP socket.
    pub fn send(&mut self) -> io::Result<()> {
        let dac_broadcast = self.create_broadcast();
        let Broadcaster {
            ref udp_socket,
            ref broadcast_addr,
            ref mut bytes,
            ..
        } = *self;
        {
            // Write the broadcast to the bytes buffer.
            let mut writer = &mut bytes[..];
            writer.write_bytes(&dac_broadcast)?;
        }
        // Send the broadcast bytes over the UDP socket.
        udp_socket.send_to(&bytes[..], broadcast_addr)?;
        Ok(())
    }

    /// Run the **Broadcaster**.
    ///
    /// The broadcaster will block on the given receiver, waiting to process **Message**s.
    ///
    /// On each **Message::Second** received, the **Broadcaster** will send a message over UDP.
    ///
    /// On each **Message::Dac**, the **Broadcaster** will update the current DAC state.
    pub fn run(&mut self, rx: Rx) -> io::Result<()> {
        // Loop forever handling each message.
        for msg in rx {
            match msg {
                // If a second has passed, send a new broadcast message.
                Message::Send => self.send()?,
                // Update the state of the DAC.
                Message::Dac(dac) => self.dac = dac,
                // Break from the loop.
                Message::Close => break,
            }
        }
        Ok(())
    }

    /// Consumes the **Broadcaster** and calls the **run** method on a separate thread.
    pub fn spawn(mut self) -> io::Result<Handle> {
        // Create the channel for sending messages to the broadcaster.
        let (tx, rx) = mpsc::channel();
        // Run the broadcaster on a new thread.
        let thread = thread::Builder::new()
            .name("ether-dream-dac-emulator".into())
            .spawn(move || {
                self.run(rx)?;
                Ok(self)
            })?;
        let thread = Arc::new(Mutex::new(Some(thread)));
        let once_per_second_timer = Arc::new(Mutex::new(None));
        Ok(Handle {
            tx,
            thread,
            once_per_second_timer,
        })
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.close_inner();
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        self.close();
    }
}

/// Spawn a thread that sends **Message::Send** to the **Broadcaster** thread once per second as
/// per the Ether Dream protocol.
pub fn spawn_once_per_second_timer(tx: Tx) -> io::Result<Timer> {
    let is_closed = Arc::new(AtomicBool::new(false));
    let is_closed2 = is_closed.clone();
    let _thread = thread::Builder::new()
        .name("ether-dream-dac-emulator-broadcaster".into())
        .spawn(move || {
            while !is_closed2.load(atomic::Ordering::Relaxed) {
                // If the channel has closed, break from the loop.
                if tx.send(Message::Send).is_err() {
                    break;
                }
                thread::sleep(time::Duration::from_secs(1));
            }
        })?;
    Ok(Timer { is_closed, _thread })
}
