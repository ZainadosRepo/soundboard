//! Receives OSC over UDP and sends playing-state feedback to a controller. Reception is off
//! until enabled, so the firewall only asks once it is wanted.

use crate::prefs::OscPrefs;
use soundboard_core::osc::{OscCommand, OscMessage};
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

pub type OnCommand = Arc<dyn Fn(OscCommand) + Send + Sync>;

#[derive(Default)]
pub struct OscService {
    listening: Mutex<Option<Arc<AtomicBool>>>,
    output: Mutex<Option<(UdpSocket, SocketAddr)>>,
    status: Arc<Mutex<String>>,
}

impl OscService {
    pub fn status(&self) -> String {
        self.status.lock().expect("status").clone()
    }

    /// Restarts reception and output with these settings.
    pub fn apply(&self, prefs: &OscPrefs, on_command: OnCommand) {
        if let Some(stop) = self.listening.lock().expect("listening").take() {
            stop.store(true, Ordering::Relaxed);
        }
        *self.output.lock().expect("output") = None;
        let set_status = |s: String| *self.status.lock().expect("status") = s;
        if !prefs.input_enabled {
            set_status("Désactivée".into());
        } else if prefs.input_port == 0 {
            set_status("Port invalide".into());
        } else {
            match UdpSocket::bind(("0.0.0.0", prefs.input_port)) {
                Ok(socket) => {
                    let stop = Arc::new(AtomicBool::new(false));
                    *self.listening.lock().expect("listening") = Some(stop.clone());
                    set_status(format!("Écoute sur le port {}", prefs.input_port));
                    let status = self.status.clone();
                    thread::Builder::new()
                        .name("osc-input".into())
                        .spawn(move || listen(socket, stop, on_command, status))
                        .expect("OSC thread starts");
                }
                Err(e) => set_status(format!("Erreur : {e}")),
            }
        }
        if prefs.output_enabled && !prefs.output_host.is_empty() && prefs.output_port > 0 {
            let target = (prefs.output_host.as_str(), prefs.output_port)
                .to_socket_addrs()
                .ok()
                .and_then(|mut a| a.next());
            match (target, UdpSocket::bind(("0.0.0.0", 0))) {
                (Some(target), Ok(socket)) => {
                    *self.output.lock().expect("output") = Some((socket, target))
                }
                _ => eprintln!(
                    "OSC output to {}:{} unavailable",
                    prefs.output_host, prefs.output_port
                ),
            }
        }
    }

    pub fn send(&self, message: &OscMessage) {
        if let Some((socket, target)) = &*self.output.lock().expect("output") {
            // UDP feedback is best effort: a controller that is off just misses it.
            if let Err(e) = socket.send_to(&message.encode(), target) {
                eprintln!("OSC send failed: {e}");
            }
        }
    }
}

fn listen(
    socket: UdpSocket,
    stop: Arc<AtomicBool>,
    on_command: OnCommand,
    status: Arc<Mutex<String>>,
) {
    // The timeout lets the thread notice it was replaced by new settings.
    if let Err(e) = socket.set_read_timeout(Some(Duration::from_millis(250))) {
        *status.lock().expect("status") = format!("Erreur : {e}");
        return;
    }
    let mut buffer = [0u8; 65_536];
    while !stop.load(Ordering::Relaxed) {
        match socket.recv_from(&mut buffer) {
            Ok((size, _)) => {
                for message in OscMessage::decode_packet(&buffer[..size]) {
                    if let Some(command) = OscCommand::parse(&message) {
                        on_command(command);
                    }
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            // Windows reports a previous send's ICMP "port unreachable" on the next receive.
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {}
            Err(e) => {
                *status.lock().expect("status") = format!("Erreur : {e}");
                return;
            }
        }
    }
}
