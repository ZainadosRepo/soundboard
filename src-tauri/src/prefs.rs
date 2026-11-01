//! App preferences in `settings.json` of the app config folder: master volume, shown set,
//! OSC, MIDI output and the global shortcut. Sets live in their own files.

use serde::{Deserialize, Serialize};
use soundboard_core::gain::MasterVolume;
use soundboard_core::Uuid;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OscPrefs {
    pub input_enabled: bool,
    pub input_port: u16,
    pub output_enabled: bool,
    pub output_host: String,
    pub output_port: u16,
}

impl Default for OscPrefs {
    fn default() -> Self {
        OscPrefs {
            input_enabled: false,
            input_port: 8080,
            output_enabled: false,
            output_host: "127.0.0.1".into(),
            output_port: 8081,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Prefs {
    pub master: MasterVolume,
    pub selected_set: Option<Uuid>,
    pub osc: OscPrefs,
    /// Name of the MIDI output that gets pad lights; `None` sends nothing.
    pub midi_output: Option<String>,
    /// Velocity of lit pads, 1…127; many pad controllers read it as a colour.
    pub midi_velocity: u8,
    /// Tauri accelerator, e.g. `Ctrl+Alt+S`.
    pub global_shortcut: Option<String>,
    #[serde(skip)]
    path: PathBuf,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            master: MasterVolume::default(),
            selected_set: None,
            osc: OscPrefs::default(),
            midi_output: None,
            midi_velocity: 127,
            global_shortcut: None,
            path: PathBuf::new(),
        }
    }
}

impl Prefs {
    /// A missing file gives the defaults; an unreadable one is moved to `settings.json.bad`
    /// and reported, so a damaged file is never silently overwritten.
    pub fn load(path: PathBuf) -> (Prefs, Option<String>) {
        let (mut prefs, warning) = match fs::read(&path) {
            Err(_) => (Prefs::default(), None),
            Ok(data) => match serde_json::from_slice::<Prefs>(&data) {
                Ok(prefs) => (prefs, None),
                Err(e) => {
                    let _ = fs::rename(&path, path.with_extension("json.bad"));
                    (
                        Prefs::default(),
                        Some(format!("Réglages illisibles ({e}) : valeurs par défaut, l’ancien fichier est gardé en settings.json.bad.")),
                    )
                }
            },
        };
        prefs.path = path;
        (prefs, warning)
    }

    /// Writes through a temporary file so a crash never leaves half a file.
    pub fn save(&self) -> Result<(), String> {
        let write = || -> std::io::Result<()> {
            if let Some(folder) = self.path.parent() {
                fs::create_dir_all(folder)?;
            }
            let tmp = self.path.with_extension("json.tmp");
            fs::write(
                &tmp,
                serde_json::to_vec_pretty(self).expect("prefs serialize"),
            )?;
            fs::rename(tmp, &self.path)
        };
        write().map_err(|e| format!("Impossible d’enregistrer les réglages : {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_keeps_damaged_files() {
        let dir = std::env::temp_dir().join(format!("soundboard-prefs-{}", Uuid::new_v4()));
        let path = dir.join("settings.json");
        let (mut prefs, warning) = Prefs::load(path.clone());
        assert!(warning.is_none());
        prefs.midi_velocity = 64;
        prefs.save().unwrap();
        assert_eq!(Prefs::load(path.clone()).0.midi_velocity, 64);

        fs::write(&path, b"{oops").unwrap();
        let (prefs, warning) = Prefs::load(path.clone());
        assert!(warning.is_some());
        assert_eq!(prefs.midi_velocity, 127);
        assert!(path.with_extension("json.bad").exists());
        fs::remove_dir_all(dir).unwrap();
    }
}
