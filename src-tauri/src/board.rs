//! The board: the library, what plays, and every action the UI, keys, MIDI, OSC and links
//! can take. Effects on the outside world (OSC and MIDI output, the window) are queued for the
//! caller, which runs them once the board is unlocked.

use crate::audio::{Engine, StartRequest};
use crate::decode;
use crate::prefs::{Prefs, PreviewPrefs};
use serde::Serialize;
use soundboard_core::gain::MasterVolume;
use soundboard_core::library::SoundLibrary;
use soundboard_core::midi::{MidiLights, MidiMessage, MidiNote};
use soundboard_core::model::{DisplayMode, PlaybackMode, SoundSet, Tile, TileColor};
use soundboard_core::osc::{OscCommand, OscMessage, SetReference, TileAction};
use soundboard_core::playback::{
    PlaybackCommand, PlaybackCoordinator, TriggerModifier, VoiceState,
};
use soundboard_core::smart::SmartRules;
use soundboard_core::store::{SetStore, PACKAGE_EXTENSION};
use soundboard_core::text::same_name;
use soundboard_core::url_command::{LinkTileAction, UrlCommand};
use soundboard_core::Uuid;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub const GRID_COLUMNS: i32 = 6;

pub enum Effect {
    Osc(OscMessage),
    Midi(Vec<MidiMessage>),
    BringForward,
    /// Sound lengths to read in the background: `(file id, path)`.
    ReadDurations(Vec<(Uuid, PathBuf)>),
}

pub struct Board {
    library: SoundLibrary,
    store: Option<SetStore>,
    coordinator: PlaybackCoordinator,
    engine: Engine,
    /// Pre-listening output; plays the preview while `prefs.preview` is on.
    cue: Engine,
    pub prefs: Prefs,
    selected_tile: Option<Uuid>,
    /// Sound lengths by file id.
    durations: HashMap<Uuid, f64>,
    /// Sets changed but not yet written: saves are grouped so dragging a slider or triggering
    /// tiles quickly doesn't rewrite the set each time.
    pending_saves: HashMap<Uuid, SoundSet>,
    pub dirty_since: Option<Instant>,
    error: Option<String>,
    midi_learning: Option<Uuid>,
    pub midi_sources: usize,
    midi_lights: MidiLights,
    /// The preview (editor listening, tile pre-listening) plays as its own voice, outside the
    /// coordinator: it never loops or solos. Holds where in the file it started.
    preview: Option<f64>,
    preview_voice: Uuid,
    effects: Vec<Effect>,
}

impl Board {
    pub fn new(
        store: Result<SetStore, String>,
        prefs: Prefs,
        engine: Engine,
        cue: Engine,
    ) -> Board {
        let mut error = None;
        let (store, sets) = match store {
            Ok(store) => match store.load_all() {
                Ok(contents) => {
                    if !contents.unreadable.is_empty() {
                        error = Some(format!(
                            "Certains sets n’ont pas pu être lus et ne sont pas affichés :\n{}",
                            contents.unreadable.join("\n")
                        ));
                    }
                    (Some(store), contents.sets)
                }
                Err(e) => {
                    error = Some(format!("Impossible d’ouvrir la bibliothèque de sons : {e}"));
                    (Some(store), vec![])
                }
            },
            Err(e) => {
                error = Some(format!("Impossible d’ouvrir la bibliothèque de sons : {e}"));
                (None, vec![])
            }
        };
        let library = SoundLibrary::new(sets, prefs.selected_set);
        engine.set_master_gain(prefs.master.gain());
        cue.set_master_gain(prefs.preview.volume);
        let mut board = Board {
            library,
            store,
            coordinator: PlaybackCoordinator::default(),
            engine,
            cue,
            prefs,
            selected_tile: None,
            durations: HashMap::new(),
            pending_saves: HashMap::new(),
            dirty_since: None,
            error,
            midi_learning: None,
            midi_sources: 0,
            midi_lights: MidiLights::default(),
            preview: None,
            preview_voice: Uuid::new_v4(),
            effects: vec![],
        };
        board.selection_changed();
        board
    }

    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    pub fn fail(&mut self, message: String) {
        self.error = Some(message);
    }

    pub fn dismiss_error(&mut self) {
        self.error = None;
    }

    fn sound_set(&self) -> &SoundSet {
        self.library.selected()
    }

    /// What the board shows: the selected set's own tiles, or for a smart set the matching
    /// tiles of ordinary sets. Hotkeys, MIDI and Play Selection act on these tiles.
    fn shown_set(&self) -> SoundSet {
        self.library.shown_set(self.sound_set(), GRID_COLUMNS)
    }

    fn is_list_mode(&self, tile_id: Uuid) -> bool {
        self.library
            .set_containing(tile_id)
            .is_some_and(|s| s.display_mode == DisplayMode::List)
    }

    // ---------- sets

    pub fn select_set(&mut self, id: Uuid) {
        self.library.select(id);
        self.selection_changed();
    }

    pub fn select_next_set(&mut self) {
        self.library.select_next();
        self.selection_changed();
    }

    pub fn select_previous_set(&mut self) {
        self.library.select_previous();
        self.selection_changed();
    }

    pub fn add_set(&mut self) {
        let set = self.library.add_set();
        self.selection_changed();
        self.save(set);
    }

    /// Adds a smart set showing favourite tiles; the UI opens its rules.
    pub fn add_smart_set(&mut self) -> Uuid {
        let mut set = self.library.add_set();
        set.title = "Smart set".into();
        set.smart_rules = Some(SmartRules::default());
        self.library.replace(set.clone());
        self.selection_changed();
        let id = set.id;
        self.save(set);
        id
    }

    pub fn rename_set(&mut self, id: Uuid, title: &str) {
        let title = title.trim();
        let Some(mut set) = self.library.set(id).cloned() else {
            return;
        };
        if title.is_empty() || set.title == title {
            return;
        }
        set.title = title.into();
        self.library.replace(set.clone());
        self.save(set);
    }

    pub fn delete_set(&mut self, id: Uuid) {
        let Some(set) = self.library.set(id).cloned() else {
            return;
        };
        if self.library.sets().len() < 2 {
            return;
        }
        // Sounds of a deleted set must not keep playing from files that are gone; going through
        // perform() also turns their pads and OSC buttons off.
        for tile in &set.tiles {
            let commands = self.coordinator.stop(tile.id, None);
            self.perform(commands);
        }
        if !self.library.remove_set(id) {
            return;
        }
        // A pending save would write the deleted set back.
        self.pending_saves.remove(&id);
        self.selection_changed();
        if let Some(Err(e)) = self.store.as_ref().map(|s| s.delete(id)) {
            self.fail(format!("Impossible de supprimer « {} » : {e}", set.title));
        }
    }

    pub fn set_rules(&mut self, id: Uuid, rules: SmartRules) {
        let Some(mut set) = self.library.set(id).cloned().filter(SoundSet::is_smart) else {
            return;
        };
        set.smart_rules = Some(rules);
        self.library.replace(set.clone());
        self.save(set);
        self.refresh_midi_lights();
    }

    /// How many tiles of ordinary sets these rules would show, while they are being edited.
    pub fn count_matching(&self, rules: SmartRules) -> usize {
        let mut probe = SoundSet::new("");
        probe.smart_rules = Some(rules);
        self.library.tile_count(&probe)
    }

    /// Writes the shown set as a `.soundset` file at `destination`.
    pub fn export_shown_set(&mut self, destination: &Path) {
        self.flush_saves();
        let set = self.sound_set().clone();
        let result = match &self.store {
            Some(store) => store.export(&set, destination).map_err(|e| e.to_string()),
            None => Err("la bibliothèque de sons n’a pas pu être ouverte.".into()),
        };
        if let Err(e) = result {
            self.fail(format!("Impossible d’exporter « {} » : {e}", set.title));
        }
    }

    /// Adds `.soundset` files to the library and shows the last one. With `reusing`, a set
    /// already in the library (opened from the file manager) is shown instead of duplicated.
    pub fn import_sets(&mut self, paths: &[PathBuf], reusing: bool) {
        let mut problems = vec![];
        for path in paths {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let Some(store) = self.store.clone() else {
                problems.push(format!("{name} : la bibliothèque n’a pas pu être ouverte."));
                continue;
            };
            if reusing {
                if let Ok(existing) = store.read_package(path) {
                    if self.library.set(existing.id).is_some() {
                        self.select_set(existing.id);
                        continue;
                    }
                }
            }
            match store.import_package(path) {
                Ok(set) => {
                    self.library.add(set);
                    self.selection_changed();
                }
                Err(e) => problems.push(format!("{name} : {e}")),
            }
        }
        if !problems.is_empty() {
            self.fail(format!(
                "Certains sets n’ont pas pu être importés.\n{}",
                problems.join("\n")
            ));
        }
    }

    // ---------- tiles

    /// Files dropped on the window: sets are opened, audio files become tiles.
    pub fn drop_files(&mut self, paths: &[PathBuf]) {
        let (sets, others): (Vec<PathBuf>, Vec<PathBuf>) = paths.iter().cloned().partition(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case(PACKAGE_EXTENSION))
        });
        if !sets.is_empty() {
            self.import_sets(&sets, true);
        }
        if !others.is_empty() {
            self.add_files(&others);
        }
    }

    /// Adds one tile per audio file to the shown set, copying the file into it.
    pub fn add_files(&mut self, paths: &[PathBuf]) {
        if self.sound_set().is_smart() {
            return self.fail(
                "Un smart set se remplit selon ses règles : ajoutez les sons dans un set normal."
                    .into(),
            );
        }
        let Some(store) = self.store.clone() else {
            return self.fail(
                "Impossible d’ajouter des sons : la bibliothèque n’a pas pu être ouverte.".into(),
            );
        };
        let audio: Vec<&PathBuf> = paths.iter().filter(|p| decode::is_audio(p)).collect();
        if audio.is_empty() {
            return self.fail("Aucun de ces fichiers n’est un son pris en charge (wav, aiff, mp3, m4a, flac, ogg).".into());
        }
        let mut set = self.sound_set().clone();
        for path in audio {
            let file_name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let title = path
                .file_stem()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let mut tile = Tile::new(Uuid::new_v4(), &file_name, &title);
            tile.grid_position = set.next_free_grid_position(GRID_COLUMNS);
            tile.list_index = set.next_list_index();
            tile.color = TileColor::ALL[set.tiles.len() % TileColor::ALL.len()];
            if let Err(e) = store.import_sound(path, &tile, set.id) {
                self.fail(format!("Impossible d’ajouter « {title} » : {e}"));
                continue;
            }
            set.tiles.push(tile);
        }
        self.library.replace(set.clone());
        self.save(set);
        self.read_missing_durations();
    }

    /// Saves an edited tile; a tile shown in a smart set keeps its own set's layout.
    pub fn update_tile(&mut self, tile: Tile) {
        let Some(mut set) = self.library.set_containing(tile.id).cloned() else {
            return;
        };
        let Some(slot) = set.tiles.iter_mut().find(|t| t.id == tile.id) else {
            return;
        };
        let mut edited = tile;
        edited.grid_position = slot.grid_position;
        edited.list_index = slot.list_index;
        edited.hotkey = slot.hotkey.clone();
        edited.midi_trigger = slot.midi_trigger;
        *slot = edited.clone();
        self.library.replace(set.clone());
        self.engine.update(edited.id, &edited.player);
        self.save(set);
    }

    fn edit_tile(&mut self, id: Uuid, change: impl FnOnce(&mut Tile)) {
        if let Some(mut tile) = self.library.tile(id).cloned() {
            change(&mut tile);
            self.update_tile(tile);
        }
    }

    pub fn select_tile(&mut self, id: Option<Uuid>) {
        self.selected_tile = id;
        self.read_missing_durations();
    }

    /// Removes the tile and its copied sound, stopping it first if it plays.
    pub fn delete_tile(&mut self, id: Uuid) {
        let Some(mut set) = self.library.set_containing(id).cloned() else {
            return;
        };
        let Some(tile) = set.tile(id).cloned() else {
            return;
        };
        let commands = self.coordinator.stop(id, None);
        self.perform(commands);
        set.tiles.retain(|t| t.id != id);
        self.library.replace(set.clone());
        if self.selected_tile == Some(id) {
            self.selected_tile = None;
        }
        let set_id = set.id;
        self.save(set);
        if let Some(Err(e)) = self.store.as_ref().map(|s| s.remove_sound(&tile, set_id)) {
            self.fail(format!(
                "Impossible de supprimer le son de « {} » : {e}",
                tile.title
            ));
        }
    }

    /// Moves a tile of the shown set to the place of `onto` in the current display order.
    pub fn move_tile(&mut self, id: Uuid, onto: Uuid) {
        if self.sound_set().is_smart() || id == onto {
            return;
        }
        let Some(index) = self
            .sound_set()
            .display_order()
            .iter()
            .position(|t| t.id == onto)
        else {
            return;
        };
        self.update_shown_set(|s| s.move_tile(id, index, GRID_COLUMNS));
        self.selected_tile = Some(id);
    }

    pub fn set_hotkey(&mut self, id: Uuid, key: Option<&str>) {
        let Some(mut set) = self.library.set_containing(id).cloned() else {
            return;
        };
        set.assign_hotkey(key, id);
        self.library.replace(set.clone());
        self.save(set);
    }

    // ---------- playback

    pub fn press(&mut self, id: Uuid, modifier: TriggerModifier, velocity: f32) {
        let Some(tile) = self.library.tile(id).cloned() else {
            return;
        };
        self.selected_tile = Some(id);
        let list_mode = self.is_list_mode(id);
        let commands = self.coordinator.press(&tile, modifier, list_mode, velocity);
        self.perform(commands);
    }

    pub fn release(&mut self, id: Uuid) {
        let Some(tile) = self.library.tile(id).cloned() else {
            return;
        };
        let list_mode = self.is_list_mode(id);
        let commands = self.coordinator.release(&tile, list_mode);
        self.perform(commands);
    }

    /// Plays (or stops) the selected tile of the shown set, e.g. the one Cue next moved to.
    pub fn play_selection(&mut self) {
        let Some(id) = self.selected_tile else {
            return;
        };
        if self.shown_set().tile(id).is_some() {
            self.press(id, TriggerModifier::None, 1.0);
        }
    }

    pub fn stop_all(&mut self) {
        let commands = self.coordinator.stop_all();
        self.perform(commands);
    }

    pub fn toggle_pause_all(&mut self) {
        let commands = self.coordinator.toggle_pause_all();
        self.perform(commands);
    }

    pub fn fade_out_all(&mut self) {
        let commands = self.coordinator.fade_out_all();
        self.perform(commands);
    }

    /// Plays the shown set's tile for `key`; false when no tile uses that key.
    pub fn hotkey(&mut self, key: &str, down: bool, shift: bool) -> bool {
        let Some(id) = self.shown_set().tile_for_hotkey(key).map(|t| t.id) else {
            return false;
        };
        if down {
            let modifier = if shift {
                TriggerModifier::Pause
            } else {
                TriggerModifier::None
            };
            self.press(id, modifier, 1.0);
        } else {
            self.release(id);
        }
        true
    }

    /// The engine reports that a voice ended on its own.
    pub fn voice_finished(&mut self, id: Uuid) {
        if id == self.preview_voice {
            self.preview = None;
            return;
        }
        self.coordinator.voice_did_finish(id);
        self.cue_next(id);
        self.send_playing_feedback(&[id]);
    }

    pub fn start_failed(&mut self, id: Uuid, error: String) {
        if id == self.preview_voice {
            self.preview = None;
        } else {
            self.coordinator.voice_did_finish(id);
            self.send_playing_feedback(&[id]);
        }
        let title = self
            .library
            .tile(id)
            .map_or("ce son".into(), |t| t.title.clone());
        self.fail(format!("Impossible de lire « {title} » : {error}"));
    }

    /// Continuous: the tile reached its crossfade point, start the next one.
    pub fn advance(&mut self, id: Uuid, lead: f64) {
        let Some(set) = self.library.set_containing(id) else {
            return;
        };
        let (Some(outgoing), Some(next)) =
            (set.tile(id).cloned(), set.next_playable_tile(id).cloned())
        else {
            return;
        };
        let commands = self.coordinator.crossfade(&outgoing, &next, lead);
        if !commands.is_empty() {
            self.selected_tile = Some(next.id);
            self.perform(commands);
        }
    }

    /// Cue next: when a tile stops, select the following one without playing it.
    fn cue_next(&mut self, id: Uuid) {
        if let Some(next) = self
            .library
            .set_containing(id)
            .and_then(|s| s.tile_to_cue(id))
        {
            self.selected_tile = Some(next.id);
        }
    }

    fn perform(&mut self, commands: Vec<PlaybackCommand>) {
        let ids: Vec<Uuid> = commands.iter().map(PlaybackCommand::tile_id).collect();
        for command in commands {
            match command {
                PlaybackCommand::Start {
                    tile,
                    fade_in,
                    velocity,
                } => self.start(tile, fade_in, velocity),
                PlaybackCommand::Stop { tile, fade } => {
                    self.engine.stop(tile, fade);
                    self.cue_next(tile);
                }
                PlaybackCommand::Pause(tile) => self.engine.pause(tile, true),
                PlaybackCommand::Resume(tile) => self.engine.pause(tile, false),
            }
        }
        self.send_playing_feedback(&ids);
    }

    fn start(&mut self, id: Uuid, fade_in: f64, velocity: f32) {
        let found = self.library.set_containing(id).and_then(|set| {
            let tile = set.tile(id)?.clone();
            let path = self.store.as_ref()?.sound_path(&tile, set.id);
            Some((tile, path, set.continuous_crossfade()))
        });
        let Some((mut tile, path, continuous_crossfade)) = found else {
            self.coordinator.voice_did_finish(id);
            return;
        };
        self.engine.start_voice(
            id,
            StartRequest {
                path,
                file_id: tile.file_id,
                settings: tile.player.clone(),
                title: tile.title.clone(),
                fade_in,
                velocity,
                continuous_crossfade,
                cached: true,
            },
        );
        tile.last_played = Some(now());
        self.update_tile(tile);
    }

    // ---------- list mode

    pub fn set_display_mode(&mut self, mode: DisplayMode) {
        self.update_shown_set(|s| s.display_mode = mode);
    }

    pub fn set_playback_mode(&mut self, mode: PlaybackMode) {
        self.update_shown_set(|s| s.playback_mode = mode);
    }

    pub fn set_crossfade(&mut self, seconds: f64) {
        self.update_shown_set(|s| s.crossfade_duration = seconds.clamp(0.0, 10.0));
    }

    /// Clears the played marks of the shown set and selects its first playable tile.
    pub fn reset_list(&mut self) {
        let shown = self.shown_set();
        for tile in shown.tiles.iter().filter(|t| t.last_played.is_some()) {
            self.edit_tile(tile.id, |t| t.last_played = None);
        }
        self.selected_tile = shown.first_playable_tile().map(|t| t.id);
    }

    fn update_shown_set(&mut self, change: impl FnOnce(&mut SoundSet)) {
        let mut set = self.sound_set().clone();
        change(&mut set);
        self.library.replace(set.clone());
        self.save(set);
    }

    // ---------- master volume

    pub fn set_master(&mut self, master: MasterVolume) {
        let mut master = master;
        master.volume_a = master.volume_a.clamp(0.0, 1.0);
        master.volume_b = master.volume_b.clamp(0.0, 1.0);
        self.prefs.master = master;
        self.engine.set_master_gain(master.gain());
        self.save_prefs();
    }

    fn edit_master(&mut self, change: impl FnOnce(&mut MasterVolume)) {
        let mut master = self.prefs.master;
        change(&mut master);
        self.set_master(master);
    }

    pub fn master_action(&mut self, action: &str) {
        match action {
            "toggleAB" => self.edit_master(MasterVolume::toggle_ab),
            "up" => self.edit_master(MasterVolume::step_up),
            "down" => self.edit_master(MasterVolume::step_down),
            "mute" => self.edit_master(|m| m.muted = !m.muted),
            _ => {}
        }
    }

    // ---------- editor

    /// Plays `start…end` of the tile's file once, as is: no loop, fades or solo.
    pub fn preview(&mut self, id: Uuid, start: f64, end: f64) {
        self.stop_preview();
        let found = self.library.set_containing(id).and_then(|set| {
            let tile = set.tile(id)?.clone();
            Some((self.store.as_ref()?.sound_path(&tile, set.id), tile))
        });
        let Some((path, tile)) = found else {
            return;
        };
        let Some(duration) = self.duration(&tile, &path) else {
            return;
        };
        let mut settings = tile.player.clone();
        settings.trim_start = start.min(end).max(0.0);
        settings.trim_end = (duration - end.max(start)).max(0.0);
        settings.loops = false;
        settings.fade_in_time = 0.0;
        settings.fade_out_time = 0.0;
        self.preview_engine().start_voice(
            self.preview_voice,
            StartRequest {
                path,
                file_id: tile.file_id,
                settings,
                title: tile.title.clone(),
                fade_in: 0.0,
                velocity: 1.0,
                continuous_crossfade: None,
                cached: false,
            },
        );
        self.preview = Some(start.min(end).max(0.0));
    }

    pub fn stop_preview(&mut self) {
        // Both: the setting may have moved the preview since it started.
        self.engine.stop(self.preview_voice, None);
        self.cue.stop(self.preview_voice, None);
        self.preview = None;
    }

    /// Pre-listening on, the preview goes to its own output and never reaches the live one.
    fn preview_engine(&self) -> &Engine {
        if self.prefs.preview.enabled {
            &self.cue
        } else {
            &self.engine
        }
    }

    /// Plays the tile's trimmed sound on the preview, or stops the preview if one plays.
    pub fn cue(&mut self, id: Uuid) {
        if self.preview.is_some() {
            self.stop_preview();
            return;
        }
        let Some(tile) = self.library.tile(id).cloned() else {
            return;
        };
        let Some(duration) = self.durations.get(&tile.file_id).copied() else {
            self.fail(format!(
                "La durée de « {} » n’est pas encore connue.",
                tile.title
            ));
            return;
        };
        self.preview(id, tile.player.trim_start, duration - tile.player.trim_end);
    }

    // ---------- audio outputs

    pub fn set_output(&mut self, device: Option<String>) {
        self.prefs.output_device = device;
        self.engine.set_output(self.prefs.output());
        self.save_prefs();
    }

    pub fn set_preview(&mut self, mut preview: PreviewPrefs) {
        self.stop_preview();
        preview.volume = preview.volume.clamp(0.0, 1.0);
        self.cue.set_master_gain(preview.volume);
        self.cue.set_output(preview.output());
        self.prefs.preview = preview;
        self.save_prefs();
    }

    /// Device in use or why none is, for the settings: (live, pre-listening).
    pub fn output_status(&self) -> (String, String) {
        (self.engine.status(), self.cue.status())
    }

    /// Moves the preview if one plays, otherwise the tile if it sounds, by `seconds`.
    pub fn scan(&mut self, id: Uuid, seconds: f64) {
        let voice = if self.preview.is_some() {
            self.preview_voice
        } else {
            id
        };
        let engine = if self.preview.is_some() {
            self.preview_engine()
        } else {
            &self.engine
        };
        if let Some((position, _)) = engine.playback(voice) {
            engine.seek(voice, position + seconds);
        }
    }

    /// Jumps the preview or the sounding tile to `time` seconds into the whole file.
    pub fn jump(&mut self, id: Uuid, time: f64) {
        if let Some(start) = self.preview {
            self.preview_engine().seek(self.preview_voice, time - start);
        } else if self.coordinator.state(id).is_some() {
            if let Some(tile) = self.library.tile(id) {
                self.engine.seek(id, time - tile.player.trim_start);
            }
        }
    }

    /// Where the preview, else the tile, plays, in seconds into the whole file.
    pub fn playhead(&self, id: Uuid) -> Option<f64> {
        if let Some(start) = self.preview {
            return self
                .preview_engine()
                .playback(self.preview_voice)
                .map(|(p, _)| start + p);
        }
        let tile = self.library.tile(id)?;
        self.engine
            .playback(id)
            .map(|(p, _)| tile.player.trim_start + p)
    }

    // ---------- links and OSC

    /// `soundboard://` links; other arguments are files to open as sets.
    pub fn open(&mut self, arguments: &[String]) {
        let mut files = vec![];
        for argument in arguments {
            if argument.to_ascii_lowercase().starts_with("soundboard:") {
                match UrlCommand::parse(argument) {
                    Some(command) => self.handle_link(command),
                    None => self.fail(format!("Lien non reconnu : {argument}")),
                }
            } else {
                let path = PathBuf::from(argument);
                if path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case(PACKAGE_EXTENSION))
                {
                    files.push(path);
                }
            }
        }
        if !files.is_empty() {
            self.import_sets(&files, true);
        }
    }

    fn handle_link(&mut self, command: UrlCommand) {
        match command {
            UrlCommand::Osc(command) => self.handle_osc(command),
            UrlCommand::ShowSet { title } => {
                match self
                    .library
                    .sets()
                    .iter()
                    .find(|s| same_name(&s.title, &title))
                {
                    Some(set) => self.select_set(set.id),
                    None => self.fail(format!("Aucun set nommé « {title} ».")),
                }
            }
            UrlCommand::Tile { title, set, action } => {
                let Some(id) = self.tile_named(&title, set.as_deref()) else {
                    let within = set
                        .map(|s| format!(" dans le set « {s} »"))
                        .unwrap_or_default();
                    return self.fail(format!("Aucune tuile nommée « {title} »{within}."));
                };
                match action {
                    // A link that plays leaves an already playing tile alone rather than stopping it.
                    LinkTileAction::Play
                        if self.coordinator.state(id) == Some(VoiceState::Playing) => {}
                    LinkTileAction::Play => self.press(id, TriggerModifier::None, 1.0),
                    LinkTileAction::Stop => {
                        let commands = self.coordinator.stop(id, None);
                        self.perform(commands);
                    }
                    LinkTileAction::Fade => {
                        let commands = self.coordinator.stop(id, Some(self.coordinator.fade_time));
                        self.perform(commands);
                    }
                }
            }
        }
    }

    /// The shown set is searched first, then the other ordinary sets in sidebar order.
    fn tile_named(&self, title: &str, set: Option<&str>) -> Option<Uuid> {
        let ordinary: Vec<&SoundSet> = self
            .library
            .sets()
            .iter()
            .filter(|s| !s.is_smart())
            .collect();
        let candidates: Vec<&SoundSet> = match set {
            Some(name) => ordinary
                .into_iter()
                .filter(|s| same_name(&s.title, name))
                .collect(),
            None => {
                let shown = self.sound_set();
                std::iter::once(shown)
                    .chain(ordinary.into_iter().filter(|s| s.id != shown.id))
                    .collect()
            }
        };
        candidates
            .iter()
            .flat_map(|s| &s.tiles)
            .find(|t| same_name(&t.title, title))
            .map(|t| t.id)
    }

    /// Runs a command received over OSC; tiles are found by set position and grid cell.
    pub fn handle_osc(&mut self, command: OscCommand) {
        match command {
            OscCommand::Tile {
                set,
                position,
                action,
                pressed,
            } => {
                let set = match set {
                    SetReference::Selected => Some(self.shown_set()),
                    SetReference::Index(i) => self
                        .library
                        .sets()
                        .get(i)
                        .map(|s| self.library.shown_set(s, GRID_COLUMNS)),
                };
                if let Some(tile) = set.and_then(|s| s.tile_at(position).cloned()) {
                    self.tile_action(&tile, action, pressed);
                }
            }
            OscCommand::MasterToggleAb => self.edit_master(MasterVolume::toggle_ab),
            OscCommand::MasterSelectVolumeA => self.edit_master(|m| m.uses_volume_b = false),
            OscCommand::MasterSelectVolumeB => self.edit_master(|m| m.uses_volume_b = true),
            OscCommand::MasterVolumeUp => self.edit_master(MasterVolume::step_up),
            OscCommand::MasterVolumeDown => self.edit_master(MasterVolume::step_down),
            OscCommand::MasterVolume(v) => self.edit_master(|m| m.set_active_volume(v)),
            OscCommand::PlayPauseAll => self.toggle_pause_all(),
            OscCommand::StopAll => self.stop_all(),
            OscCommand::BringForward => self.effects.push(crate::board::Effect::BringForward),
        }
    }

    fn tile_action(&mut self, tile: &Tile, action: TileAction, pressed: bool) {
        match action {
            TileAction::Play if pressed => self.press(tile.id, TriggerModifier::None, 1.0),
            TileAction::Play => self.release(tile.id),
            _ if !pressed => {}
            // Fades a playing tile out; an idle tile has nothing to fade.
            TileAction::Fade => {
                if self.coordinator.state(tile.id).is_some() {
                    self.press(tile.id, TriggerModifier::Fade, 1.0);
                }
            }
            TileAction::Loop => self.edit_tile(tile.id, |t| t.player.loops = !t.player.loops),
            TileAction::Mute => self.edit_tile(tile.id, |t| t.player.muted = !t.player.muted),
            TileAction::Solo => self.edit_tile(tile.id, |t| t.solo = !t.solo),
            TileAction::ToggleAb => self.edit_tile(tile.id, |t| {
                t.player.uses_volume_b = !t.player.uses_volume_b
            }),
        }
    }

    /// Tells the OSC controller which of these tiles play, so it can light their buttons.
    fn send_playing_feedback(&mut self, ids: &[Uuid]) {
        self.refresh_midi_lights();
        if !self.prefs.osc.output_enabled {
            return;
        }
        let mut seen = BTreeSet::new();
        for id in ids.iter().filter(|id| seen.insert(**id)) {
            let Some(set) = self.library.set_containing(*id) else {
                continue;
            };
            let Some(index) = self.library.sets().iter().position(|s| s.id == set.id) else {
                continue;
            };
            let Some(tile) = set.tile(*id) else {
                continue;
            };
            let playing = self.coordinator.state(*id) == Some(VoiceState::Playing);
            self.effects.push(Effect::Osc(OscCommand::playing_feedback(
                index,
                tile.grid_position,
                playing,
            )));
        }
    }

    // ---------- MIDI

    /// The next note played on a controller will trigger this tile.
    pub fn start_midi_learn(&mut self, id: Uuid) {
        self.selected_tile = Some(id);
        self.midi_learning = Some(id);
    }

    pub fn cancel_midi_learn(&mut self) {
        self.midi_learning = None;
    }

    pub fn clear_midi(&mut self, id: Uuid) {
        self.set_midi(id, None);
    }

    /// Note-on plays the shown set's tile for that note, scaled by velocity; note-off releases
    /// Hold to Play tiles. While learning, the first note-on goes to the learning tile.
    pub fn handle_midi(&mut self, message: MidiMessage) {
        match message {
            MidiMessage::NoteOn(note, velocity) => {
                if let Some(learning) = self.midi_learning.take() {
                    return self.set_midi(learning, Some(note));
                }
                if let Some(id) = self.shown_set().tile_for_midi(note).map(|t| t.id) {
                    self.press(
                        id,
                        TriggerModifier::None,
                        MidiMessage::gain_for_velocity(velocity),
                    );
                }
            }
            MidiMessage::NoteOff(note) => {
                if let Some(id) = self.shown_set().tile_for_midi(note).map(|t| t.id) {
                    self.release(id);
                }
            }
        }
    }

    fn set_midi(&mut self, id: Uuid, note: Option<MidiNote>) {
        let Some(mut set) = self.library.set_containing(id).cloned() else {
            return;
        };
        set.assign_midi(note, id);
        self.library.replace(set.clone());
        self.save(set);
        self.refresh_midi_lights();
    }

    /// Lights the pads of the shown set's playing tiles; the shown set's notes are the ones the
    /// controller triggers, so switching sets relights the pads.
    fn refresh_midi_lights(&mut self) {
        let wanted: BTreeSet<MidiNote> = self
            .shown_set()
            .tiles
            .iter()
            .filter(|t| self.coordinator.state(t.id) == Some(VoiceState::Playing))
            .filter_map(|t| t.midi_trigger)
            .collect();
        let messages = self.midi_lights.update(&wanted, self.prefs.midi_velocity);
        if !messages.is_empty() {
            self.effects.push(Effect::Midi(messages));
        }
    }

    /// Pads lit under the previous MIDI output are turned off before switching.
    pub fn relight_midi(&mut self) -> Vec<MidiMessage> {
        self.midi_lights
            .update(&BTreeSet::new(), self.prefs.midi_velocity)
    }

    pub fn after_midi_output_change(&mut self) {
        self.refresh_midi_lights();
    }

    // ---------- durations and waveforms

    fn duration(&mut self, tile: &Tile, path: &Path) -> Option<f64> {
        if let Some(d) = self.durations.get(&tile.file_id) {
            return Some(*d);
        }
        let d = decode::duration(path).ok()?;
        self.durations.insert(tile.file_id, d);
        Some(d)
    }

    pub fn set_duration(&mut self, file_id: Uuid, seconds: f64) {
        self.durations.insert(file_id, seconds);
    }

    /// Asks the caller to read the lengths of the shown set's sounds (and the selected tile's).
    fn read_missing_durations(&mut self) {
        let Some(store) = &self.store else {
            return;
        };
        let mut tiles = self.shown_set().tiles;
        if let Some(selected) = self.selected_tile.and_then(|id| self.library.tile(id)) {
            tiles.push(selected.clone());
        }
        let missing: Vec<(Uuid, PathBuf)> = tiles
            .iter()
            .filter(|t| !self.durations.contains_key(&t.file_id))
            .filter_map(|t| {
                let set = self.library.set_containing(t.id)?;
                Some((t.file_id, store.sound_path(t, set.id)))
            })
            .collect();
        if !missing.is_empty() {
            self.effects.push(Effect::ReadDurations(missing));
        }
    }

    /// What a waveform needs, read without holding the board: the store, the tile, its set
    /// and its sound file.
    pub fn waveform_source(&self, id: Uuid) -> Option<(SetStore, Tile, Uuid, PathBuf)> {
        let store = self.store.clone()?;
        let set = self.library.set_containing(id)?;
        let tile = set.tile(id)?.clone();
        let path = store.sound_path(&tile, set.id);
        Some((store, tile, set.id, path))
    }

    // ---------- persistence

    pub fn save_prefs(&mut self) {
        if let Err(e) = self.prefs.save() {
            self.fail(e);
        }
    }

    /// Called on every change of the shown set.
    fn selection_changed(&mut self) {
        self.prefs.selected_set = Some(self.library.selected_id());
        self.save_prefs();
        self.refresh_midi_lights();
        self.read_missing_durations();
    }

    /// Writes the set shortly, grouping quick successive changes into one write.
    fn save(&mut self, set: SoundSet) {
        self.pending_saves.insert(set.id, set);
        self.dirty_since.get_or_insert_with(Instant::now);
    }

    /// Writes every pending set now; also called when the app quits.
    pub fn flush_saves(&mut self) {
        self.dirty_since = None;
        let sets: Vec<SoundSet> = self.pending_saves.drain().map(|(_, s)| s).collect();
        for set in sets {
            if let Some(Err(e)) = self.store.as_ref().map(|s| s.save(&set)) {
                self.fail(format!("Impossible d’enregistrer « {} » : {e}", set.title));
            }
        }
    }

    // ---------- what the UI shows

    pub fn snapshot(&self) -> Snapshot {
        let shown = self.shown_set();
        let order = shown.display_order();
        let tiles = order
            .iter()
            .map(|t| TileView {
                state: self.coordinator.state(t.id),
                duration: self.durations.get(&t.file_id).copied(),
                tile: (*t).clone(),
            })
            .collect();
        let selected = self
            .selected_tile
            .and_then(|id| self.library.tile(id))
            .map(|t| TileView {
                state: self.coordinator.state(t.id),
                duration: self.durations.get(&t.file_id).copied(),
                tile: t.clone(),
            });
        Snapshot {
            sets: self
                .library
                .sets()
                .iter()
                .map(|s| SetSummary {
                    id: s.id,
                    title: s.title.clone(),
                    smart: s.is_smart(),
                    count: self.library.tile_count(s),
                })
                .collect(),
            set: SetView {
                id: shown.id,
                title: shown.title.clone(),
                display_mode: shown.display_mode,
                playback_mode: shown.playback_mode,
                crossfade_duration: shown.crossfade_duration,
                smart_rules: shown.smart_rules.clone(),
                tiles,
            },
            selected,
            master: self.prefs.master,
            midi_learning: self.midi_learning,
            midi_sources: self.midi_sources,
            previewing: self.preview.is_some(),
            pre_listening: self.prefs.preview.enabled,
            error: self.error.clone(),
            can_delete_set: self.library.sets().len() > 1,
        }
    }
}

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    sets: Vec<SetSummary>,
    set: SetView,
    selected: Option<TileView>,
    master: MasterVolume,
    midi_learning: Option<Uuid>,
    midi_sources: usize,
    previewing: bool,
    /// Pre-listening output on: tiles offer « Pré-écouter ».
    pre_listening: bool,
    error: Option<String>,
    can_delete_set: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SetSummary {
    id: Uuid,
    title: String,
    smart: bool,
    count: usize,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SetView {
    id: Uuid,
    title: String,
    display_mode: DisplayMode,
    playback_mode: PlaybackMode,
    crossfade_duration: f64,
    smart_rules: Option<SmartRules>,
    /// In display order.
    tiles: Vec<TileView>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TileView {
    #[serde(flatten)]
    tile: Tile,
    state: Option<VoiceState>,
    duration: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{EngineEvent, Output};
    use std::sync::mpsc::Receiver;
    use std::time::Duration;

    fn board(dir: &Path, master_muted: bool) -> (Board, Receiver<EngineEvent>) {
        let (engine, events) = Engine::start(Output::System, true);
        let (cue, _) = Engine::start(Output::Off, false);
        let (mut prefs, _) = Prefs::load(dir.join("settings.json"));
        prefs.master.muted = master_muted;
        let board = Board::new(Ok(SetStore::new(dir.join("Sets"))), prefs, engine, cue);
        (board, events)
    }

    fn next_event(events: &Receiver<EngineEvent>) -> EngineEvent {
        events
            .recv_timeout(Duration::from_secs(10))
            .expect("an audio event")
    }

    /// Plays real files through the default output, muted: needs a sound card, so it is left
    /// out of CI. `cargo test -p soundboard -- --ignored`.
    #[test]
    #[ignore = "needs an audio output"]
    fn plays_and_chains_real_sounds() {
        let dir = tempfile::tempdir().unwrap();
        let (mut board, events) = board(dir.path(), true);
        let files: Vec<PathBuf> = (0..2)
            .map(|i| {
                let path = dir.path().join(format!("son {i}.wav"));
                decode::tests::write_wav(&path, 0.6, 44_100, 2);
                path
            })
            .collect();
        board.add_files(&files);
        assert!(board.error.is_none(), "{:?}", board.error);
        let ids: Vec<Uuid> = board
            .sound_set()
            .list_order()
            .iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(ids.len(), 2);

        board.press(ids[0], TriggerModifier::None, 1.0);
        assert_eq!(board.coordinator.state(ids[0]), Some(VoiceState::Playing));
        match next_event(&events) {
            EngineEvent::Finished(id) => board.voice_finished(id),
            _ => panic!("the sound should just end"),
        }
        assert_eq!(board.coordinator.state(ids[0]), None);
        assert!(board.library.tile(ids[0]).unwrap().last_played.is_some());

        board.set_display_mode(DisplayMode::List);
        board.set_playback_mode(PlaybackMode::Continuous);
        board.set_crossfade(0.2);
        board.press(ids[0], TriggerModifier::None, 1.0);
        let EngineEvent::Advance(id, lead) = next_event(&events) else {
            panic!("Continuous should reach the advance point first");
        };
        assert_eq!((id, lead), (ids[0], 0.2));
        board.advance(id, lead);
        assert_eq!(board.coordinator.state(ids[1]), Some(VoiceState::Playing));
        assert_eq!(board.selected_tile, Some(ids[1]));
        let mut finished = vec![];
        while finished.len() < 2 {
            if let EngineEvent::Finished(id) = next_event(&events) {
                board.voice_finished(id);
                finished.push(id);
            }
        }
        assert!(board.coordinator.states().is_empty());

        board.flush_saves();
        let reloaded = SetStore::new(dir.path().join("Sets")).load_all().unwrap();
        assert_eq!(reloaded.sets[0].tiles.len(), 2);
        assert_eq!(reloaded.sets[0].playback_mode, PlaybackMode::Continuous);
    }

    #[test]
    #[ignore = "needs an audio output"]
    fn unreadable_sounds_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let (mut board, events) = board(dir.path(), true);
        let fake = dir.path().join("faux.mp3");
        std::fs::write(&fake, b"pas un son").unwrap();
        board.add_files(&[fake]);
        let id = board.sound_set().tiles[0].id;
        board.press(id, TriggerModifier::None, 1.0);
        match next_event(&events) {
            EngineEvent::StartFailed(id, e) => board.start_failed(id, e),
            _ => panic!("the start should fail"),
        }
        assert_eq!(board.coordinator.state(id), None);
        assert!(board
            .error
            .as_deref()
            .unwrap()
            .contains("Impossible de lire « faux »"));
    }
}
