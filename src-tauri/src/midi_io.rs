//! MIDI controllers: every input is listened to (notes trigger tiles), and one chosen output
//! gets pad lights. Ports are rescanned so controllers can be plugged in at any time.

use midir::{MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
use soundboard_core::midi::MidiMessage;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const CLIENT: &str = "Soundboard";

pub type OnMessage = Arc<dyn Fn(MidiMessage) + Send + Sync>;
pub type OnSources = Arc<dyn Fn(usize) + Send + Sync>;

#[derive(Default)]
pub struct Midi {
    inputs: Mutex<HashMap<String, MidiInputConnection<()>>>,
    output: Mutex<Option<(String, MidiOutputConnection)>>,
}

impl Midi {
    /// Connects to every input now and every few seconds after, reporting how many are open.
    pub fn watch_inputs(self: &Arc<Self>, on_message: OnMessage, on_sources: OnSources) {
        let midi = self.clone();
        thread::Builder::new()
            .name("midi-inputs".into())
            .spawn(move || loop {
                let count = midi.rescan(&on_message);
                on_sources(count);
                thread::sleep(Duration::from_secs(3));
            })
            .expect("MIDI thread starts");
    }

    fn rescan(&self, on_message: &OnMessage) -> usize {
        let Ok(probe) = MidiInput::new(CLIENT) else {
            return 0;
        };
        let available: Vec<String> = probe
            .ports()
            .iter()
            .filter_map(|p| probe.port_name(p).ok())
            .collect();
        let mut inputs = self.inputs.lock().expect("inputs");
        inputs.retain(|name, _| available.contains(name));
        let missing: Vec<&String> = available
            .iter()
            .filter(|n| !inputs.contains_key(*n))
            .collect();
        for name in missing {
            let Ok(input) = MidiInput::new(CLIENT) else {
                continue;
            };
            let Some(port) = input
                .ports()
                .into_iter()
                .find(|p| input.port_name(p).ok().as_deref() == Some(name))
            else {
                continue;
            };
            let on_message = on_message.clone();
            let connected = input.connect(
                &port,
                "soundboard-in",
                move |_, bytes, _| {
                    if let Some(message) = MidiMessage::parse(bytes) {
                        on_message(message);
                    }
                },
                (),
            );
            match connected {
                Ok(connection) => {
                    inputs.insert(name.clone(), connection);
                }
                Err(e) => eprintln!("MIDI input {name} unavailable: {e}"),
            }
        }
        inputs.len()
    }

    pub fn output_names() -> Vec<String> {
        MidiOutput::new(CLIENT)
            .map(|out| {
                out.ports()
                    .iter()
                    .filter_map(|p| out.port_name(p).ok())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Opens the output named `name`, or closes it with `None`.
    pub fn set_output(&self, name: Option<&str>) -> Result<(), String> {
        let mut output = self.output.lock().expect("output");
        *output = None;
        let Some(name) = name else {
            return Ok(());
        };
        let out = MidiOutput::new(CLIENT).map_err(|e| e.to_string())?;
        let port = out
            .ports()
            .into_iter()
            .find(|p| out.port_name(p).ok().as_deref() == Some(name))
            .ok_or_else(|| format!("l’appareil MIDI « {name} » est débranché"))?;
        let connection = out
            .connect(&port, "soundboard-out")
            .map_err(|e| e.to_string())?;
        *output = Some((name.to_owned(), connection));
        Ok(())
    }

    pub fn send(&self, messages: &[MidiMessage]) {
        if let Some((name, connection)) = &mut *self.output.lock().expect("output") {
            for message in messages {
                if let Err(e) = connection.send(&message.bytes()) {
                    eprintln!("MIDI send to {name} failed: {e}");
                }
            }
        }
    }
}
