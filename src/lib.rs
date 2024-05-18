//! # bevy_serial
//!
//! `bevy_serial` is a plugin to add non-blocking serial communication to bevy. This plugin is based on [`mio-serial`](https://github.com/berkowski/mio-serial) that can realize non-blocking high-performance I/O.
//!
//! Reading and writing from/to serial port is realized via bevy's event system. Each serial port is handled via port name or a unique label you choose. These event handlers are added to the following stage to minimize the frame delay.
//!
//! - Reading: `PreUpdate`
//! - Writing: `PostUpdate`
//!
//! ## Usage
//!
//! ### Simple Example
//!
//! Here is a simple example:
//!
//! ```rust
//! use bevy::prelude::*;
//! use bevy_serial::{SerialPlugin, SerialReadEvent, SerialWriteEvent};
//!
//! // to write data to serial port periodically
//! #[derive(Resource)]
//! struct SerialWriteTimer(Timer);
//!
//! const SERIAL_PORT: &str = "/dev/ttyUSB0";
//!
//! fn main() {
//!     App::new()
//!         .add_plugins(MinimalPlugins)
//!         // simply specify port name and baud rate for `SerialPlugin`
//!         .add_plugins(SerialPlugin::new(SERIAL_PORT, 115200))
//!         // to write data to serial port periodically (every 1 second)
//!         .insert_resource(SerialWriteTimer(Timer::from_seconds(
//!             1.0,
//!             TimerMode::Repeating,
//!         )))
//!         // reading and writing from/to serial port is achieved via bevy's event system
//!         .add_systems(Update, read_serial)
//!         .add_systems(Update, write_serial)
//!         .run();
//! }
//!
//! // reading event for serial port
//! fn read_serial(mut ev_serial: EventReader<SerialReadEvent>) {
//!     // you can get label of the port and received data buffer from `SerialReadEvent`
//!     for SerialReadEvent(label, buffer) in ev_serial.iter() {
//!         let s = String::from_utf8(buffer.clone()).unwrap();
//!         println!("received packet from {label}: {s}");
//!     }
//! }
//!
//! // writing event for serial port
//! fn write_serial(
//!     mut ev_serial: EventWriter<SerialWriteEvent>,
//!     mut timer: ResMut<SerialWriteTimer>,
//!     time: Res<Time>,
//! ) {
//!     // write msg to serial port every 1 second not to flood serial port
//!     if timer.0.tick(time.delta()).just_finished() {
//!         // you can write to serial port via `SerialWriteEvent` with label and buffer to write
//!         let buffer = b"Hello, bevy!";
//!         ev_serial.send(SerialWriteEvent(SERIAL_PORT.to_string(), buffer.to_vec()));
//!     }
//! }
//! ```
//!
//! ### Multiple Serial Ports with Additional Settings
//!
//! You can add multiple serial ports with additional settings.
//!
//! ```rust
//! fn main() {
//!     App::new()
//!         .add_plugins(MinimalPlugins)
//!         // you can specify various configurations for multiple serial ports by this way
//!         .add_plugins(SerialPlugin {
//!             settings: vec![SerialSetting {
//!                 label: Some(SERIAL_LABEL.to_string()),
//!                 port_name: SERIAL_PORT.to_string(),
//!                 baud_rate: 115200,
//!                 data_bits: DataBits::Eight,
//!                 flow_control: FlowControl::None,
//!                 parity: Parity::None,
//!                 stop_bits: StopBits::One,
//!                 timeout: Duration::from_millis(0),
//!             }],
//!         })
//!         // reading and writing from/to serial port is achieved via bevy's event system
//!         .add_systems(Update, read_serial)
//!         .add_systems(Update, write_serial)
//!         .run();
//! }
//! ```
//!
//! ## Supported Versions
//!
//! | bevy  | bevy_serial |
//! | ----- | ----------- |
//! | 0.13  | 0.5         |
//! | 0.12  | 0.4         |
//! | 0.11  | 0.3         |
//! | 0.6   | 0.2         |
//! | 0.5   | 0.1         |
//!
//! ## License
//!
//! Dual-licensed under either
//!
//! - MIT
//! - Apache 2.0

pub use mio_serial::{DataBits, FlowControl, Parity, StopBits};

use bevy::app::{App, Plugin, PostUpdate, PreUpdate};
use bevy::ecs::event::{Event, EventReader, EventWriter};
use bevy::ecs::system::{In, IntoSystem, Res, ResMut, Resource};
use mio::{Events, Interest, Poll, Token};
use mio_serial::SerialStream;
use once_cell::sync::OnceCell;
use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Plugin that can be added to Bevy
pub struct SerialPlugin {
    pub settings: Vec<SerialSetting>,
    pub on_read_error: Arc<dyn Fn(In<std::io::Result<()>>) + Send + Sync>,
    pub on_write_error: Arc<dyn Fn(In<std::io::Result<()>>) + Send + Sync>,
}

impl SerialPlugin {
    pub fn new(
        port_name: &str,
        baud_rate: u32,
        on_read_error: Arc<dyn Fn(In<std::io::Result<()>>) + Send + Sync>,
        on_write_error: Arc<dyn Fn(In<std::io::Result<()>>) + Send + Sync>,
    ) -> Self {
        Self {
            settings: vec![SerialSetting {
                port_name: port_name.to_string(),
                baud_rate,
                ..Default::default()
            }],
            on_read_error,
            on_write_error,
        }
    }
}

/// Settings for users to initialize this plugin
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerialSetting {
    /// The intuitive name for this serial port
    pub label: Option<String>,
    /// The port name, usually the device path
    pub port_name: String,
    /// The baud rate in symbols-per-second
    pub baud_rate: u32,
    /// Number of bits used to represent a character sent on the line
    pub data_bits: DataBits,
    /// The type of signalling to use for controlling data transfer
    pub flow_control: FlowControl,
    /// The type of parity to use for error checking
    pub parity: Parity,
    /// Number of bits to use to signal the end of a character
    pub stop_bits: StopBits,
    /// Amount of time to wait to receive data before timing out
    pub timeout: Duration,
}

impl Default for SerialSetting {
    fn default() -> Self {
        Self {
            label: None,
            port_name: "".to_string(),
            baud_rate: 115200,
            data_bits: DataBits::Eight,
            flow_control: FlowControl::None,
            parity: Parity::None,
            stop_bits: StopBits::One,
            timeout: Duration::from_millis(0),
        }
    }
}

/// Bevy's event type to read serial port
#[derive(Event)]
pub struct SerialReadEvent(pub String, pub Vec<u8>);

/// Bevy's event type to read serial port
#[derive(Event)]
pub struct SerialWriteEvent(pub String, pub Vec<u8>);

/// Serial struct that is used internally for this crate
#[derive(Debug)]
struct SerialStreamLabeled {
    stream: SerialStream,
    label: String,
    connected: bool,
}

/// Module scope global singleton to store serial ports
static SERIALS: OnceCell<Vec<Mutex<SerialStreamLabeled>>> = OnceCell::new();

/// Context to poll serial read event with `Poll` in `mio` crate
#[derive(Resource)]
struct MioContext {
    poll: Poll,
    events: Events,
}

impl MioContext {
    /// poll serial read event (should timeout not to block other systems)
    fn poll(&mut self) {
        self.poll
            .poll(&mut self.events, Some(Duration::from_micros(1)))
            .unwrap_or_else(|e| {
                panic!("Failed to poll events: {e:?}");
            });
    }
}

/// Component to get an index of serial port based on the label
#[derive(Resource)]
struct Indices(HashMap<String, usize>);

/// The size of read buffer for one read system call
const DEFAULT_READ_BUFFER_LEN: usize = 2048;

impl Plugin for SerialPlugin {
    fn build(&self, app: &mut App) {
        let poll = Poll::new().unwrap();
        let events = Events::with_capacity(self.settings.len());
        let mio_ctx = MioContext { poll, events };
        let mut serials: Vec<Mutex<SerialStreamLabeled>> = vec![];
        let mut indices = Indices(HashMap::new());

        for (i, setting) in self.settings.iter().enumerate() {
            // create serial port builder from `serialport` crate
            let port_builder = serialport::new(&setting.port_name, setting.baud_rate)
                .data_bits(setting.data_bits)
                .flow_control(setting.flow_control)
                .parity(setting.parity)
                .stop_bits(setting.stop_bits)
                .timeout(setting.timeout);

            // create `mio_serial::SerailStream` from `seriaport` builder
            let mut stream = SerialStream::open(&port_builder).unwrap_or_else(|e| {
                panic!("Failed to open serial port {}\n{:?}", setting.port_name, e);
            });

            // token index is same as index of vec
            mio_ctx
                .poll
                .registry()
                .register(&mut stream, Token(i), Interest::READABLE)
                .unwrap_or_else(|e| {
                    panic!("Failed to register stream to poll : {e:?}");
                });

            // if label is set, use label as a nickname of serial
            // if not, use `port_name` as a nickname
            let label = if let Some(label) = &setting.label {
                label.clone()
            } else {
                setting.port_name.clone()
            };

            // store indices and serials
            indices.0.insert(label.clone(), i);
            serials.push(Mutex::new(SerialStreamLabeled {
                stream,
                label,
                connected: true,
            }));
        }

        // set to global variables lazily
        SERIALS.set(serials).unwrap_or_else(|e| {
            panic!("Failed to set SerialStream to global variable: {e:?}");
        });

        app.insert_resource(mio_ctx)
            .insert_resource(indices)
            .add_event::<SerialReadEvent>()
            .add_event::<SerialWriteEvent>()
            .add_systems(PreUpdate, read_serial.pipe(self.on_read_error))
            .add_systems(PostUpdate, write_serial.pipe(self.on_write_error));
    }
}

/// Poll serial read event with `Poll` in `mio` crate.
/// If any data has come to serial, `SerialReadEvent` is sent to the system subscribing it.
fn read_serial(
    mut ev_receive_serial: EventWriter<SerialReadEvent>,
    mut mio_ctx: ResMut<MioContext>,
    indices: Res<Indices>,
) -> std::io::Result<()> {
    if !indices.0.is_empty() {
        // poll serial read events
        mio_ctx.poll();

        // if events have occurred, send `SerialReadEvent` with serial labels and read data buffer
        for event in mio_ctx.events.iter() {
            // get serial instance based on the token index
            let serials = SERIALS.get().expect("SERIALS are not initialized");
            let serial_mtx = serials
                .get(event.token().0) // token index is same as index of vec
                .expect("SERIALS are not initialized");

            if event.is_readable() {
                let mut buffer = vec![0_u8; DEFAULT_READ_BUFFER_LEN];
                let mut bytes_read = 0;
                loop {
                    // try to get lock of mutex and send data to event
                    if let Ok(mut serial) = serial_mtx.lock() {
                        if serial.connected {
                            match serial.stream.read(&mut buffer[bytes_read..]) {
                                Ok(0) => {
                                    eprintln!("read connection closed");
                                    serial.connected = false;
                                    break;
                                }
                                // read data successfully
                                // if buffer is full, maybe there is more data to read
                                Ok(n) => {
                                    bytes_read += n;
                                    if bytes_read == buffer.len() {
                                        buffer.resize(buffer.len() + DEFAULT_READ_BUFFER_LEN, 0);
                                    }
                                }
                                // would block indicates no more data to read
                                Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                                    let label = serial.label.clone();
                                    let buffer = buffer.drain(..bytes_read).collect();
                                    ev_receive_serial.send(SerialReadEvent(label, buffer));
                                    break;
                                }
                                // if interrupted, we should continue readings
                                Err(ref e) if e.kind() == ErrorKind::Interrupted => {
                                    continue;
                                }
                                // other errors are fatal
                                Err(e) => {
                                    eprintln!("Failed to read serial port {}: {}", serial.label, e);
                                }
                            }
                        } else {
                            eprintln!("{} connection has closed", serial.label);
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// Write bytes to serial port.
/// The bytes are sent via `SerialWriteEvent` with label of serial port.
fn write_serial(
    mut ev_write_serial: EventReader<SerialWriteEvent>,
    indices: Res<Indices>,
) -> std::io::Result<()> {
    if !indices.0.is_empty() {
        for SerialWriteEvent(label, buffer) in ev_write_serial.read() {
            // get index of label
            let &serial_index = indices.0.get(label).unwrap_or_else(|| {
                panic!("Label {} is not exist", label.as_str());
            });
            let serials = SERIALS.get().expect("SERIALS are not initialized");
            let serial_mtx = serials
                .get(serial_index)
                .expect("SERIALS are not initialized");

            // write buffered data to serial
            let mut bytes_wrote = 0;
            loop {
                // try to get lock of mutex and send data to event
                if let Ok(mut serial) = serial_mtx.lock() {
                    if serial.connected {
                        // write the entire buffered data in a single system call
                        match serial.stream.write(&buffer[bytes_wrote..]) {
                            // error if returned len is less than expected (same as `io::Write::write_all` does)
                            Ok(n) if n < buffer.len() => {
                                eprintln!(
                                    "write size error {} / {}",
                                    n,
                                    buffer.len() - bytes_wrote
                                );
                                bytes_wrote += n;
                            }
                            // wrote queued data successfully
                            Ok(_) => {
                                bytes_wrote += buffer.len();
                            }
                            // would block indicates that this port is not ready so try again
                            Err(ref e) if e.kind() == ErrorKind::WouldBlock => {}
                            // if interrupted, we should try again
                            Err(ref e) if e.kind() == ErrorKind::Interrupted => {}
                            // other errors are fatal
                            Err(e) => {
                                eprintln!("Failed to write serial port {}: {}", serial.label, e);
                            }
                        }
                    } else {
                        eprintln!("{} connection has closed", serial.label);
                    }

                    if bytes_wrote == buffer.len() {
                        break;
                    } else {
                        continue;
                    }
                }
            }
        }
    }
    Ok(())
}
