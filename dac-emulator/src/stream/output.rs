use crossbeam::sync::{MsQueue, SegQueue};
use ether_dream::{dac, protocol};
use ether_dream::protocol::command;
use std::collections::VecDeque;
use std::{fmt, mem, ops, time, thread};
use std::error::Error;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{self, AtomicUsize};

// A message queue for allowingn the **Stream** to communicate with its **output::Processor**.
type MessageQueue = MsQueue<Message>;

// The queue used for receving updates to the point rate.
type PointRateQueue = SegQueue<command::PointRate>;

// The queue used to move processed data from the stream to output.
type DataQueue = MsQueue<Vec<protocol::DacPoint>>;

// The queue used to move messages from the output processor to the **Output** handle.
type OutputMessageQueue = MsQueue<OutputMessage>;

// // A queue for returning used buffers to the **Stream** so that they may be re-used.
type UsedBufferQueue = SegQueue<Vec<protocol::DacPoint>>;

/// The state of the output processor thread.
///
/// The **output::Processor** receives point rates and point data queued by the associated
/// **Stream**.  Each **Stream** always has a single associated **output::Processor** to which it
/// feeds data.
///
/// The **output::Processor** thread is responsible for pulling data and point rates from the
/// queues as necessary and forwards them in "frames" to the **Output** at the specified point and
/// frame rates.
pub struct Processor {
    // The target number of points emitted by the processor per second.
    point_rate: u32,
    // The target number of frames emitted by the processor per second.
    frame_rate: u32,

    // Communication with the **Stream**.
    stream_shared: Arc<StreamShared>,

    // Communication with the **Output** handle.
    output_message_queue: Arc<OutputMessageQueue>,
    output_used_buffer_queue: Arc<UsedBufferQueue>,
}

/// A handle for receiving the **Output** produced by the processor.
///
/// This type may be used to yield "frames" of **DacPoint**s emitted by the **Processor**.
#[derive(Clone)]
pub struct Output {
    message_queue: Arc<OutputMessageQueue>,
    used_buffer_queue: Arc<UsedBufferQueue>,
}

// Messages that may be received by the **Output** handle.
enum OutputMessage {
    // New data to yield.
    Data(Vec<protocol::DacPoint>),
    // The stream has closed.
    Close,
}

/// A **Frame** of **DacPoint**s emitted by the DAC.
///
/// **Frame** implemented **Deref** targeting an inner slice of **DacPoint**s.
///
/// Once dropped, the frame returns the buffer to the **DAC** so that it may be re-used.
pub struct Frame {
    points: Vec<protocol::DacPoint>,
    used_buffer_queue: Arc<UsedBufferQueue>,
}

/// The error returned when an **Output** can no longer yield frames as the stream is closed.
#[derive(Clone, Debug)]
pub struct StreamClosed;

impl Output {
    /// Yields the next **Frame** if there is one pending.
    ///
    /// Once the yielded **Frame** is dropped, it sends the buffer back to the DAC for reuse.
    ///
    /// Returns `None` if there are no pending frames.
    pub fn try_next_frame(&self) -> Result<Option<Frame>, StreamClosed> {
        match self.message_queue.try_pop() {
            None => Ok(None),
            Some(OutputMessage::Data(points)) => {
                let used_buffer_queue = self.used_buffer_queue.clone();
                Ok(Some(Frame { points, used_buffer_queue }))
            }
            Some(OutputMessage::Close) => Err(StreamClosed),
        }
    }

    /// Yields the next **Frame**.
    ///
    /// If there are no **Frame**s queue in the inner buffer, this method will block until there is
    /// one.
    ///
    /// Once the yielded **Frame** is dropped, it sends the buffer back to the DAC for reuse.
    ///
    /// Returns `None` if there are no pending frames.
    pub fn next_frame(&self) -> Result<Frame, StreamClosed> {
        match self.message_queue.pop() {
            OutputMessage::Data(points) => {
                let used_buffer_queue = self.used_buffer_queue.clone();
                Ok(Frame { points, used_buffer_queue })
            }
            OutputMessage::Close => Err(StreamClosed),
        }
    }
}

/// A handle to the output processor.
///
/// This is used by the **Stream** to communicate with the output processor.
pub struct Handle {
    shared: Arc<StreamShared>,
    /// An output instance.
    output: Output,
    thread: thread::JoinHandle<()>,
}

// Data shared between an output processor and its handle.
struct StreamShared {
    message_queue: MessageQueue,
    data_queue: DataQueue,
    point_rate_queue: PointRateQueue,
    used_buffer_queue: UsedBufferQueue,
    buffer_fullness: AtomicUsize,
    point_count: AtomicUsize,
}

// Messages that may be sent from a **Stream** to its associated **output::Processor**.
enum Message {
    Begin(command::Begin),
    Stop,
    Close,
}

impl Handle {
    /// Push data to the queue for processing.
    ///
    /// Returns the resulting number of points in the buffer.
    pub fn push_data(&self, data: Vec<protocol::DacPoint>) -> usize {
        let data_len = data.len();
        let prev_len = self.shared.buffer_fullness.fetch_add(data_len, atomic::Ordering::Relaxed);
        self.shared.data_queue.push(data);
        data_len + prev_len
    }

    /// Push a point rate change to the queue for processing.
    pub fn push_point_rate(&self, point_rate: command::PointRate) {
        self.shared.point_rate_queue.push(point_rate);
    }

    /// Send a message to indicate that processing should begin.
    pub fn begin(&self, begin: command::Begin) {
        self.shared.message_queue.push(Message::Begin(begin));
    }

    /// Send a message to indicate that processing should stop and buffers should be cleared.
    pub fn stop(&self) {
        self.shared.message_queue.push(Message::Stop);
    }

    /// Send a message to fininsh processing and break from the loop.
    pub fn close(&self) {
        self.close_inner();
    }

    /// Produce an **Output** for the processor for yielding frames of points emitted by the
    /// **Processor**.
    ///
    /// **Note** that all **Output**s produced by calling this method will share the same data
    /// queue. In turn each frame emitted by the **Processor** will only be yielded via the
    /// **Output** that first calls **next_frame**.
    pub fn output(&self) -> Output {
        self.output.clone()
    }

    /// Retrieve the number of points currently buffered.
    pub fn buffer_fullness(&self) -> usize {
        self.shared.buffer_fullness.load(atomic::Ordering::Relaxed)
    }

    /// Retrieve the total number of points emitted by the DAC since it last started playing.
    pub fn point_count(&self) -> usize {
        self.shared.point_count.load(atomic::Ordering::Relaxed)
    }

    fn close_inner(&self) {
        self.shared.message_queue.push(Message::Close);
    }
}

impl Processor {
    /// Create a new **Processor** that is ready to run at the given **frame_rate**.
    pub fn new(frame_rate: u32) -> Self {
        // Create the struct to be shared with the stream.
        let message_queue = MessageQueue::new();
        let data_queue = DataQueue::new();
        let point_rate_queue = PointRateQueue::new();
        let used_buffer_queue = SegQueue::new();
        let buffer_fullness = AtomicUsize::new(0);
        let point_count = AtomicUsize::new(0);
        let stream_shared = Arc::new(StreamShared {
            message_queue,
            data_queue,
            point_rate_queue,
            used_buffer_queue,
            buffer_fullness,
            point_count,
        });

        // Initialise the point rate to `0`.
        let point_rate = 0;

        // The queues for communicating with the **Output**.
        let output_message_queue = Arc::new(OutputMessageQueue::new());
        let output_used_buffer_queue = Arc::new(SegQueue::new());

        // TODO: Should probably pre-allocate a couple buffers each for the stream and output here.

        Processor {
            point_rate,
            frame_rate,
            stream_shared,
            output_message_queue,
            output_used_buffer_queue,
        }
    }

    /// Produce an **Output** for the processor for yielding frames of points emitted by the
    /// **Processor**.
    ///
    /// **Note** that all **Output**s produced by calling this method will share the same data
    /// queue. In turn each frame emitted by the **Processor** will only be yielded via the
    /// **Output** that first calls **next_frame**.
    pub fn output(&self) -> Output {
        let message_queue = self.output_message_queue.clone();
        let used_buffer_queue = self.output_used_buffer_queue.clone();
        Output { message_queue, used_buffer_queue }
    }

    /// Run the output processor, initialised with the given point and frame rates.
    pub fn run(&mut self) {
        let Processor {
            ref mut frame_rate,
            ref mut point_rate,
            ref stream_shared,
            ref output_message_queue,
            ref output_used_buffer_queue,
        } = *self;

        // Data is pushed to the back and popped from the front.
        let mut unprocessed_points = VecDeque::new();
        let mut unprocessed_point_rates = VecDeque::new();

        // Wait to receive messages from the **Stream**.
        loop {
            match stream_shared.message_queue.pop() {
                // Begin the frame output loop.
                Message::Begin(begin) => {
                    *point_rate = begin.point_rate;
                    match run_output(
                        *frame_rate,
                        point_rate,
                        stream_shared,
                        output_message_queue,
                        output_used_buffer_queue,
                        &mut unprocessed_points,
                        &mut unprocessed_point_rates,
                    ) {
                        Some(Close) => break,
                        _ => continue,
                    }
                },

                // Nothing to be done for **Stop** as we're not currently running anyway.
                Message::Stop => {
                    stream_shared.point_count.store(0, atomic::Ordering::Relaxed);
                    stream_shared.buffer_fullness
                        .fetch_sub(unprocessed_points.len(), atomic::Ordering::Relaxed);
                    unprocessed_points.clear();
                    unprocessed_point_rates.clear();
                },

                // If the thread was closed, break from the loop.
                Message::Close => break,
            }
        }

        // Indicate to the output handle that the stream is now closed.
        output_message_queue.push(OutputMessage::Close);
    }

    /// Run the **output::Processor** on a separate thread and return a **Handle** for
    /// communicating with it.
    pub fn spawn(mut self) -> io::Result<Handle> {
        let output = self.output();
        let shared = self.stream_shared.clone();
        let thread = thread::Builder::new()
            .name("ether-dream-dac-emulator-stream-output-processor".into())
            .spawn(move || {
                self.run();
            })?;
        let handle = Handle { shared, output, thread };
        Ok(handle)
    }
}

struct Close;

// The output processing loop - entered when a **Begin** command is received.
//
// Returns whether or not the thread has been signalled to close closed.
fn run_output(
    frame_rate: u32,
    point_rate: &mut u32,
    stream_shared: &StreamShared,
    output_message_queue: &OutputMessageQueue,
    output_used_buffer_queue: &UsedBufferQueue,
    unprocessed_points: &mut VecDeque<protocol::DacPoint>,
    unprocessed_point_rates: &mut VecDeque<u32>,
) -> Option<Close> {
    loop {
        // Check to see if we've received any messages for breaking from the loop.
        while let Some(msg) = stream_shared.message_queue.try_pop() {
            match msg {
                Message::Begin(_) => continue,
                Message::Stop => {
                    stream_shared.point_count.store(0, atomic::Ordering::Relaxed);
                    stream_shared.buffer_fullness.fetch_sub(
                        unprocessed_points.len(),
                        atomic::Ordering::Relaxed,
                    );
                    unprocessed_points.clear();
                    unprocessed_point_rates.clear();
                },
                Message::Close => return Some(Close),
            }
        }

        // Determine the initial `points_per_frame` for this frame.
        let mut points_per_frame = (*point_rate / frame_rate) as usize;

        // Retrieve a buffer to use for collecting points for the frame.
        let mut frame_points = output_used_buffer_queue
            .try_pop()
            .unwrap_or_else(|| Vec::with_capacity(points_per_frame));
        frame_points.clear();

        // Collect any data that is pending processing.
        while let Some(mut data) = stream_shared.data_queue.try_pop() {
            for point in data.drain(..) {
                unprocessed_points.push_back(point);
            }
            stream_shared.used_buffer_queue.push(data);
        }

        // Fill the buffer one point at a time, checking for rate changes.
        let mut count = 0;
        while count < points_per_frame {
            match unprocessed_points.pop_front() {
                None => {
                    eprintln!("output processor underflowed");
                    break;
                }
                Some(point) => {
                    // If the points per second should be updated, do so.
                    match dac::PointControl::from_bits(point.control) {
                        Some(dac::PointControl::CHANGE_RATE) => {
                            if let Some(new_rate) = unprocessed_point_rates.pop_front() {
                                let frame_fract = count as f32 / points_per_frame as f32;
                                *point_rate = new_rate;
                                points_per_frame = (*point_rate / frame_rate) as usize;
                                count = (frame_fract * points_per_frame as f32) as _;
                            }
                        },
                        _ => (),
                    }
                    frame_points.push(point);
                }
            }
            count += 1;
        }

        // Push the frame to the output queue.
        stream_shared.buffer_fullness.fetch_sub(frame_points.len(), atomic::Ordering::Relaxed);
        stream_shared.point_count.fetch_add(frame_points.len(), atomic::Ordering::Relaxed);
        output_message_queue.push(OutputMessage::Data(frame_points));

        // Sleep for the duration of a single frame.
        let sleep_interval = time::Duration::from_millis((1_000 / frame_rate) as _);
        thread::sleep(sleep_interval);
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.close_inner();
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        let points = mem::replace(&mut self.points, Vec::new());
        self.used_buffer_queue.push(points);
    }
}

impl ops::Deref for Frame {
    type Target = [protocol::DacPoint];
    fn deref(&self) -> &Self::Target {
        &self.points[..]
    }
}

impl Error for StreamClosed {
    fn description(&self) -> &str {
        "the stream has closed"
    }
}

impl fmt::Display for StreamClosed {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.description())
    }
}
