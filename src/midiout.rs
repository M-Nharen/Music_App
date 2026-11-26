//! Part 4b: virtual MIDI out. Everything you hear (your notes, chords,
//! drums, the looper and optionally the song backing) is also sent as MIDI
//! to a port called "Qwertone" that other apps can record or play with
//! their own sounds (a DAW, a softsynth, a lighting app…).
//!
//! The port is an ALSA sequencer client. PipeWire picks ALSA sequencer
//! ports up through its MIDI bridge, so it shows up there too (qpwgraph,
//! Helvum, and JACK apps running on pipewire-jack).
//!
//! The audio thread never talks to ALSA: it `try_send`s small fixed-size
//! messages, stamped with the moment their sound starts, to a thread here,
//! which waits until that moment and sends them. So MIDI keeps the same
//! sample-accurate spacing as the audio, and a slow MIDI client can never
//! cause an audio glitch.

use crossbeam_channel::{Receiver, Sender};
use std::time::{Duration, Instant};

/// One short MIDI message from the audio thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MidiMsg {
    /// When the matching sound starts (its buffer position).
    pub at: Instant,
    pub data: [u8; 3],
}

impl MidiMsg {
    pub fn note_on(at: Instant, ch: u8, note: u8, vel: u8) -> Self {
        MidiMsg {
            at,
            data: [0x90 | (ch & 15), note & 127, vel.clamp(1, 127)],
        }
    }
    pub fn note_off(at: Instant, ch: u8, note: u8) -> Self {
        MidiMsg {
            at,
            data: [0x80 | (ch & 15), note & 127, 0],
        }
    }
    pub fn program(at: Instant, ch: u8, program: u8) -> Self {
        MidiMsg {
            at,
            data: [0xC0 | (ch & 15), program & 127, 0],
        }
    }
    pub fn control(at: Instant, ch: u8, cc: u8, value: u8) -> Self {
        MidiMsg {
            at,
            data: [0xB0 | (ch & 15), cc & 127, value & 127],
        }
    }
}

/// Where the messages finally go (ALSA, or a list in tests).
pub trait Sink: Send {
    fn send(&mut self, data: [u8; 3]) -> Result<(), String>;
}

/// Messages queued between the audio thread and the sender thread.
const QUEUE: usize = 4096;
/// Don't wait longer than this for a message's time (clock hiccups).
const MAX_WAIT: Duration = Duration::from_millis(100);

/// The running MIDI out. Dropping it closes the port.
pub struct MidiOut {
    tx: Sender<MidiMsg>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// e.g. "Qwertone:out (client 128)".
    pub name: String,
}

impl MidiOut {
    /// Open the ALSA sequencer port.
    pub fn start() -> Result<MidiOut, String> {
        let (sink, name) = alsa_sink::AlsaSink::open("Qwertone")?;
        Ok(Self::with_sink(Box::new(sink), name))
    }

    pub fn with_sink(sink: Box<dyn Sink>, name: String) -> MidiOut {
        let (tx, rx) = crossbeam_channel::bounded(QUEUE);
        let thread = std::thread::Builder::new()
            .name("qwertone-midi".into())
            .spawn(move || run(rx, sink))
            .ok();
        MidiOut { tx, thread, name }
    }

    /// A sender for the audio thread.
    pub fn sender(&self) -> Sender<MidiMsg> {
        self.tx.clone()
    }
}

impl Drop for MidiOut {
    fn drop(&mut self) {
        // The thread ends once every sender is gone; the audio thread drops
        // its copy when told (`Command::MidiOut(None)`), so don't wait here.
        let (dead, _) = crossbeam_channel::bounded(1);
        drop(std::mem::replace(&mut self.tx, dead));
        if let Some(t) = self.thread.take() {
            std::thread::spawn(move || {
                let _ = t.join();
            });
        }
    }
}

fn run(rx: Receiver<MidiMsg>, mut sink: Box<dyn Sink>) {
    while let Ok(m) = rx.recv() {
        let now = Instant::now();
        if m.at > now {
            std::thread::sleep((m.at - now).min(MAX_WAIT));
        }
        let _ = sink.send(m.data);
    }
    // Port closing: let other apps release anything still held.
    for ch in 0..16u8 {
        let _ = sink.send([0xB0 | ch, 123, 0]);
    }
}

#[cfg(target_os = "linux")]
mod alsa_sink {
    use super::Sink;
    use alsa::seq::{EvCtrl, EvNote, Event, EventType, PortCap, PortType, Seq};
    use std::ffi::CString;

    pub struct AlsaSink {
        seq: Seq,
        port: i32,
    }

    impl AlsaSink {
        pub fn open(client: &str) -> Result<(AlsaSink, String), String> {
            let err = |what: &str, e: alsa::Error| format!("{what}: {e}");
            let seq = Seq::open(None, Some(alsa::Direction::Playback), false).map_err(|e| {
                format!(
                    "couldn't open the ALSA sequencer ({e}). Is the snd-seq kernel module loaded? \
                     Try: sudo modprobe snd-seq"
                )
            })?;
            let name = CString::new(client).unwrap_or_default();
            seq.set_client_name(&name)
                .map_err(|e| err("naming the client", e))?;
            let port = seq
                .create_simple_port(
                    &CString::new("out").unwrap_or_default(),
                    PortCap::READ | PortCap::SUBS_READ,
                    PortType::MIDI_GENERIC | PortType::APPLICATION | PortType::SOFTWARE,
                )
                .map_err(|e| err("creating the port", e))?;
            let id = seq.client_id().unwrap_or(-1);
            Ok((
                AlsaSink { seq, port },
                format!("{client}:out (ALSA client {id}, port {port})"),
            ))
        }
    }

    impl Sink for AlsaSink {
        fn send(&mut self, d: [u8; 3]) -> Result<(), String> {
            let ch = d[0] & 15;
            let mut ev = match d[0] & 0xF0 {
                0x90 => Event::new(
                    EventType::Noteon,
                    &EvNote {
                        channel: ch,
                        note: d[1],
                        velocity: d[2],
                        off_velocity: 0,
                        duration: 0,
                    },
                ),
                0x80 => Event::new(
                    EventType::Noteoff,
                    &EvNote {
                        channel: ch,
                        note: d[1],
                        velocity: 0,
                        off_velocity: 0,
                        duration: 0,
                    },
                ),
                0xB0 => Event::new(
                    EventType::Controller,
                    &EvCtrl {
                        channel: ch,
                        param: d[1] as u32,
                        value: d[2] as i32,
                    },
                ),
                0xC0 => Event::new(
                    EventType::Pgmchange,
                    &EvCtrl {
                        channel: ch,
                        param: 0,
                        value: d[1] as i32,
                    },
                ),
                _ => return Ok(()),
            };
            ev.set_source(self.port);
            ev.set_subs();
            ev.set_direct();
            self.seq
                .event_output_direct(&mut ev)
                .map(|_| ())
                .map_err(|e| e.to_string())
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod alsa_sink {
    pub struct AlsaSink;
    impl AlsaSink {
        pub fn open(_: &str) -> Result<(AlsaSink, String), String> {
            Err("virtual MIDI out needs Linux (ALSA sequencer)".into())
        }
    }
    impl super::Sink for AlsaSink {
        fn send(&mut self, _: [u8; 3]) -> Result<(), String> {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    type Log = Arc<Mutex<Vec<(Instant, [u8; 3])>>>;
    struct ListSink(Log);
    impl Sink for ListSink {
        fn send(&mut self, d: [u8; 3]) -> Result<(), String> {
            self.0.lock().unwrap().push((Instant::now(), d));
            Ok(())
        }
    }

    #[test]
    fn messages_go_out_in_order_at_their_time() {
        let got = Arc::new(Mutex::new(vec![]));
        let out = MidiOut::with_sink(Box::new(ListSink(got.clone())), "test".into());
        let tx = out.sender();
        let t0 = Instant::now() + Duration::from_millis(20);
        tx.send(MidiMsg::program(t0, 0, 4)).unwrap();
        tx.send(MidiMsg::note_on(t0, 0, 60, 100)).unwrap();
        tx.send(MidiMsg::note_off(t0 + Duration::from_millis(30), 0, 60))
            .unwrap();
        drop(tx);
        drop(out);
        std::thread::sleep(Duration::from_millis(150));
        let got = got.lock().unwrap();
        let data: Vec<[u8; 3]> = got.iter().map(|(_, d)| *d).collect();
        assert_eq!(&data[..3], &[[0xC0, 4, 0], [0x90, 60, 100], [0x80, 60, 0]]);
        // Then "all notes off" on every channel as the port closes.
        assert_eq!(data.len(), 3 + 16);
        assert_eq!(data[3], [0xB0, 123, 0]);
        assert!(got[1].0 >= t0, "waited for its time");
        assert!(got[2].0 >= t0 + Duration::from_millis(30));
    }

    #[test]
    fn encoding() {
        let t = Instant::now();
        assert_eq!(MidiMsg::note_on(t, 17, 200, 0).data, [0x91, 72, 1]);
        assert_eq!(MidiMsg::note_off(t, 9, 36).data, [0x89, 36, 0]);
        assert_eq!(MidiMsg::control(t, 2, 123, 0).data, [0xB2, 123, 0]);
    }
}
