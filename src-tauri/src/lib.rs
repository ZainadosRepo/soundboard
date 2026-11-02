mod audio;
mod board;
mod decode;
mod midi_io;
mod osc_service;
mod prefs;
mod update;

use audio::{Engine, EngineEvent};
use board::{Board, Effect};
use midi_io::Midi;
use osc_service::OscService;
use prefs::{OscPrefs, Prefs, PreviewPrefs};
use serde::Serialize;
use soundboard_core::gain::MasterVolume;
use soundboard_core::hotkey::is_valid_global_shortcut;
use soundboard_core::model::{DisplayMode, PlaybackMode, Tile};
use soundboard_core::playback::TriggerModifier;
use soundboard_core::smart::SmartRules;
use soundboard_core::store::{SetStore, PACKAGE_EXTENSION};
use soundboard_core::waveform::{self, Peak};
use soundboard_core::Uuid;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, WindowEvent};
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

struct App {
    board: Mutex<Board>,
    osc: OscService,
    midi: Arc<Midi>,
    handle: AppHandle,
}

type Shared = Arc<App>;

impl App {
    /// Runs `change` on the board, then its effects, then shows the new state.
    fn with<R>(self: &Arc<Self>, change: impl FnOnce(&mut Board) -> R) -> R {
        let (result, effects, snapshot) = {
            let mut board = self.board.lock().expect("board");
            let result = change(&mut board);
            (result, board.take_effects(), board.snapshot())
        };
        self.finish(effects, snapshot);
        result
    }

    /// Kept out of the generic `with`: the duration thread calls back into `with`, which would
    /// otherwise instantiate itself without end.
    fn finish(self: &Arc<Self>, effects: Vec<Effect>, snapshot: board::Snapshot) {
        for effect in effects {
            match effect {
                Effect::Osc(message) => self.osc.send(&message),
                Effect::Midi(messages) => self.midi.send(&messages),
                Effect::BringForward => bring_forward(&self.handle),
                Effect::ReadDurations(files) => {
                    let app = self.clone();
                    thread::spawn(move || app.read_durations(files));
                }
            }
        }
        if let Err(e) = self.handle.emit("board", snapshot) {
            eprintln!("board update not sent: {e}");
        }
    }

    fn read_durations(self: &Arc<Self>, files: Vec<(Uuid, PathBuf)>) {
        let found: Vec<(Uuid, f64)> = files
            .iter()
            // Unreadable files are reported where they are used (playback, editor).
            .filter_map(|(id, path)| decode::duration(path).ok().map(|d| (*id, d)))
            .collect();
        self.with(|b| found.into_iter().for_each(|(id, d)| b.set_duration(id, d)));
    }

    fn apply_osc(self: &Arc<Self>) {
        let prefs = self.board.lock().expect("board").prefs.osc.clone();
        let app = self.clone();
        self.osc.apply(
            &prefs,
            Arc::new(move |command| app.with(|b| b.handle_osc(command))),
        );
    }
}

fn bring_forward(handle: &AppHandle) {
    if let Some(window) = handle.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Command-line arguments that are set files (a double-clicked `.soundset`).
fn set_files(arguments: impl IntoIterator<Item = String>) -> Vec<String> {
    arguments
        .into_iter()
        .filter(|a| {
            PathBuf::from(a)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case(PACKAGE_EXTENSION))
        })
        .collect()
}

// ---------- commands

type Cmd<'a> = State<'a, Shared>;

#[tauri::command]
fn snapshot(app: Cmd) -> board::Snapshot {
    app.board.lock().expect("board").snapshot()
}

#[tauri::command]
fn dismiss_error(app: Cmd) {
    app.with(Board::dismiss_error);
}

#[tauri::command]
fn select_set(app: Cmd, id: Uuid) {
    app.with(|b| b.select_set(id));
}

#[tauri::command]
fn step_set(app: Cmd, forward: bool) {
    app.with(|b| {
        if forward {
            b.select_next_set()
        } else {
            b.select_previous_set()
        }
    });
}

#[tauri::command]
fn add_set(app: Cmd) {
    app.with(Board::add_set);
}

#[tauri::command]
fn add_smart_set(app: Cmd) -> Uuid {
    app.with(Board::add_smart_set)
}

#[tauri::command]
fn rename_set(app: Cmd, id: Uuid, title: String) {
    app.with(|b| b.rename_set(id, &title));
}

#[tauri::command]
fn delete_set(app: Cmd, id: Uuid) {
    app.with(|b| b.delete_set(id));
}

#[tauri::command]
fn set_rules(app: Cmd, id: Uuid, rules: SmartRules) {
    app.with(|b| b.set_rules(id, rules));
}

#[tauri::command]
fn count_matching(app: Cmd, rules: SmartRules) -> usize {
    app.board.lock().expect("board").count_matching(rules)
}

#[tauri::command]
fn set_display_mode(app: Cmd, mode: DisplayMode) {
    app.with(|b| b.set_display_mode(mode));
}

#[tauri::command]
fn set_playback_mode(app: Cmd, mode: PlaybackMode) {
    app.with(|b| b.set_playback_mode(mode));
}

#[tauri::command]
fn set_crossfade(app: Cmd, seconds: f64) {
    app.with(|b| b.set_crossfade(seconds));
}

#[tauri::command]
fn reset_list(app: Cmd) {
    app.with(Board::reset_list);
}

#[tauri::command]
async fn choose_sounds(app: Cmd<'_>, handle: AppHandle) -> Result<(), String> {
    let picked = handle
        .dialog()
        .file()
        .set_title("Ajouter des sons")
        .add_filter("Sons", &decode::AUDIO_EXTENSIONS)
        .blocking_pick_files();
    let paths: Vec<PathBuf> = picked
        .unwrap_or_default()
        .into_iter()
        .filter_map(|p| p.into_path().ok())
        .collect();
    if !paths.is_empty() {
        app.with(|b| b.add_files(&paths));
    }
    Ok(())
}

#[tauri::command]
async fn import_sets(app: Cmd<'_>, handle: AppHandle) -> Result<(), String> {
    let picked = handle
        .dialog()
        .file()
        .set_title("Importer des sets")
        .add_filter("Sets Soundboard", &[PACKAGE_EXTENSION])
        .blocking_pick_files();
    let paths: Vec<PathBuf> = picked
        .unwrap_or_default()
        .into_iter()
        .filter_map(|p| p.into_path().ok())
        .collect();
    if !paths.is_empty() {
        app.with(|b| b.import_sets(&paths, false));
    }
    Ok(())
}

#[tauri::command]
async fn export_set(app: Cmd<'_>, handle: AppHandle, title: String) -> Result<(), String> {
    let picked = handle
        .dialog()
        .file()
        .set_title("Exporter le set")
        .set_file_name(format!("{title}.{PACKAGE_EXTENSION}"))
        .add_filter("Set Soundboard", &[PACKAGE_EXTENSION])
        .blocking_save_file();
    if let Some(path) = picked.and_then(|p| p.into_path().ok()) {
        app.with(|b| b.export_shown_set(&path));
    }
    Ok(())
}

#[tauri::command]
fn press(app: Cmd, id: Uuid, modifier: TriggerModifier) {
    app.with(|b| b.press(id, modifier, 1.0));
}

#[tauri::command]
fn release(app: Cmd, id: Uuid) {
    app.with(|b| b.release(id));
}

#[tauri::command]
fn transport(app: Cmd, action: String) {
    app.with(|b| match action.as_str() {
        "stopAll" => b.stop_all(),
        "pauseAll" => b.toggle_pause_all(),
        "fadeOutAll" => b.fade_out_all(),
        "playSelection" => b.play_selection(),
        _ => {}
    });
}

#[tauri::command]
fn hotkey(app: Cmd, key: String, down: bool, shift: bool) -> bool {
    app.with(|b| b.hotkey(&key, down, shift))
}

#[tauri::command]
fn select_tile(app: Cmd, id: Option<Uuid>) {
    app.with(|b| b.select_tile(id));
}

#[tauri::command]
fn update_tile(app: Cmd, tile: Tile) {
    app.with(|b| b.update_tile(tile));
}

#[tauri::command]
fn delete_tile(app: Cmd, id: Uuid) {
    app.with(|b| b.delete_tile(id));
}

#[tauri::command]
fn move_tile(app: Cmd, id: Uuid, onto: Uuid) {
    app.with(|b| b.move_tile(id, onto));
}

#[tauri::command]
fn set_hotkey(app: Cmd, id: Uuid, key: Option<String>) {
    app.with(|b| b.set_hotkey(id, key.as_deref()));
}

#[tauri::command]
fn midi_learn(app: Cmd, id: Option<Uuid>) {
    app.with(|b| match id {
        Some(id) => b.start_midi_learn(id),
        None => b.cancel_midi_learn(),
    });
}

#[tauri::command]
fn clear_midi(app: Cmd, id: Uuid) {
    app.with(|b| b.clear_midi(id));
}

#[tauri::command]
fn set_master(app: Cmd, master: MasterVolume) {
    app.with(|b| b.set_master(master));
}

#[tauri::command]
fn master_action(app: Cmd, action: String) {
    app.with(|b| b.master_action(&action));
}

#[tauri::command]
fn preview(app: Cmd, id: Uuid, start: f64, end: f64) {
    app.with(|b| b.preview(id, start, end));
}

#[tauri::command]
fn stop_preview(app: Cmd) {
    app.with(Board::stop_preview);
}

#[tauri::command]
fn scan(app: Cmd, id: Uuid, seconds: f64) {
    app.with(|b| b.scan(id, seconds));
}

#[tauri::command]
fn jump(app: Cmd, id: Uuid, time: f64) {
    app.with(|b| b.jump(id, time));
}

/// Polled while the editor shows a sounding tile; does not redraw the board.
#[tauri::command]
fn playhead(app: Cmd, id: Uuid) -> Option<f64> {
    app.board.lock().expect("board").playhead(id)
}

/// Peaks read from the set's cache, or computed from the sound and cached for next time.
#[tauri::command]
async fn waveform(app: Cmd<'_>, id: Uuid, editor: bool) -> Result<Vec<Peak>, String> {
    let (store, tile, set_id, path) = app
        .board
        .lock()
        .expect("board")
        .waveform_source(id)
        .ok_or("Cette tuile n’existe plus.")?;
    tauri::async_runtime::spawn_blocking(move || {
        compute_waveform(&store, &tile, set_id, &path, editor)
    })
    .await
    .map_err(|e| e.to_string())?
}

fn compute_waveform(
    store: &SetStore,
    tile: &Tile,
    set_id: Uuid,
    path: &std::path::Path,
    editor: bool,
) -> Result<Vec<Peak>, String> {
    let bins = if editor {
        waveform::editor_bin_count(decode::duration(path)?)
    } else {
        waveform::BIN_COUNT
    };
    if let Some(saved) = store.saved_waveform(tile, set_id, bins) {
        return Ok(saved);
    }
    let peaks = waveform::peaks(&decode::mono(path)?, bins);
    // Not saving only costs recomputing at the next launch; the waveform still shows.
    if let Err(e) = store.save_waveform(&peaks, tile, set_id, bins) {
        eprintln!("waveform not saved: {e}");
    }
    Ok(peaks)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Settings {
    outputs: Vec<String>,
    output_device: Option<String>,
    output_status: String,
    preview: PreviewPrefs,
    preview_status: String,
    osc: OscPrefs,
    osc_status: String,
    midi_outputs: Vec<String>,
    midi_output: Option<String>,
    midi_velocity: u8,
    midi_sources: usize,
    global_shortcut: Option<String>,
}

#[tauri::command]
fn settings(app: Cmd) -> Settings {
    let board = app.board.lock().expect("board");
    let (output_status, preview_status) = board.output_status();
    Settings {
        outputs: audio::output_names(),
        output_device: board.prefs.output_device.clone(),
        output_status,
        preview: board.prefs.preview.clone(),
        preview_status,
        osc: board.prefs.osc.clone(),
        osc_status: app.osc.status(),
        midi_outputs: Midi::output_names(),
        midi_output: board.prefs.midi_output.clone(),
        midi_velocity: board.prefs.midi_velocity,
        midi_sources: board.midi_sources,
        global_shortcut: board.prefs.global_shortcut.clone(),
    }
}

#[tauri::command]
fn set_output(app: Cmd, device: Option<String>) {
    app.with(|b| b.set_output(device));
}

#[tauri::command]
fn set_preview(app: Cmd, preview: PreviewPrefs) {
    app.with(|b| b.set_preview(preview));
}

#[tauri::command]
fn cue(app: Cmd, id: Uuid) {
    app.with(|b| b.cue(id));
}

#[tauri::command]
fn set_osc(app: Cmd, osc: OscPrefs) -> String {
    app.with(|b| {
        b.prefs.osc = osc;
        b.save_prefs();
    });
    app.apply_osc();
    app.osc.status()
}

#[tauri::command]
fn set_midi_output(app: Cmd, name: Option<String>, velocity: u8) -> Result<(), String> {
    let off = app.with(|b| b.relight_midi());
    app.midi.send(&off);
    let opened = app.midi.set_output(name.as_deref());
    app.with(|b| {
        b.prefs.midi_output = name;
        b.prefs.midi_velocity = velocity.clamp(1, 127);
        b.save_prefs();
        b.after_midi_output_change();
    });
    opened
}

#[tauri::command]
fn set_global_shortcut(
    app: Cmd,
    handle: AppHandle,
    accelerator: Option<String>,
) -> Result<(), String> {
    if let Some(a) = &accelerator {
        if !is_valid_global_shortcut(a) {
            return Err("Ajoutez Ctrl, Alt ou la touche Windows/Commande.".into());
        }
    }
    let shortcuts = handle.global_shortcut();
    shortcuts.unregister_all().map_err(|e| e.to_string())?;
    if let Some(a) = &accelerator {
        shortcuts.register(a.as_str()).map_err(|e| {
            format!("Raccourci indisponible ({e}) : il est peut-être déjà pris par une autre app.")
        })?;
    }
    app.with(|b| {
        b.prefs.global_shortcut = accelerator;
        b.save_prefs();
    });
    Ok(())
}

// ---------- startup

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|handle, argv, _| {
            bring_forward(handle);
            // Links reach the deep-link handler; only set files are left here.
            let files = set_files(argv.into_iter().skip(1));
            if !files.is_empty() {
                handle.state::<Shared>().with(|b| b.open(&files));
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|handle, _, event| {
                    if event.state == ShortcutState::Pressed {
                        bring_forward(handle);
                    }
                })
                .build(),
        )
        .setup(|tauri_app| {
            let handle = tauri_app.handle().clone();
            // SOUNDBOARD_LIBRARY_DIR points the app at another library, e.g. a throwaway one
            // for checks that must not touch the real sets.
            let store = match std::env::var_os("SOUNDBOARD_LIBRARY_DIR") {
                Some(dir) => Ok(SetStore::new(PathBuf::from(dir))),
                None => handle
                    .path()
                    .app_data_dir()
                    .map(|d| SetStore::new(d.join("Sets")))
                    .map_err(|e| e.to_string()),
            };
            let prefs_path = handle.path().app_config_dir()?.join("settings.json");
            let (prefs, warning) = Prefs::load(prefs_path);
            let (engine, events) = Engine::start(prefs.output(), true);
            // No fallback: a missing headphone output must not send pre-listening live.
            let (cue, cue_events) = Engine::start(prefs.preview.output(), false);
            let shortcut = prefs.global_shortcut.clone();
            let midi_output = prefs.midi_output.clone();
            let mut board = Board::new(store, prefs, engine, cue);
            if let Some(w) = warning {
                board.fail(w);
            }
            let app: Shared = Arc::new(App {
                board: Mutex::new(board),
                osc: OscService::default(),
                midi: Arc::new(Midi::default()),
                handle: handle.clone(),
            });
            tauri_app.manage(app.clone());

            let events_app = app.clone();
            thread::Builder::new()
                .name("audio-events".into())
                .spawn(move || {
                    for event in events {
                        events_app.with(|b| match event {
                            EngineEvent::Finished(id) => b.voice_finished(id),
                            EngineEvent::Advance(id, lead) => b.advance(id, lead),
                            EngineEvent::StartFailed(id, e) => b.start_failed(id, e),
                        });
                    }
                })?;
            let cue_app = app.clone();
            thread::Builder::new()
                .name("preview-events".into())
                .spawn(move || {
                    for event in cue_events {
                        cue_app.with(|b| match event {
                            EngineEvent::Finished(id) => b.voice_finished(id),
                            EngineEvent::StartFailed(id, e) => b.start_failed(id, e),
                            EngineEvent::Advance(..) => {}
                        });
                    }
                })?;

            let saver = app.clone();
            thread::Builder::new()
                .name("saver".into())
                .spawn(move || loop {
                    thread::sleep(Duration::from_millis(200));
                    let due = saver
                        .board
                        .lock()
                        .expect("board")
                        .dirty_since
                        .is_some_and(|t| t.elapsed() >= Duration::from_millis(500));
                    if due {
                        saver.with(Board::flush_saves);
                    }
                })?;

            app.apply_osc();
            let (on_message, on_sources) = (app.clone(), app.clone());
            app.midi.watch_inputs(
                Arc::new(move |message| on_message.with(|b| b.handle_midi(message))),
                Arc::new(move |count| {
                    let changed = on_sources.board.lock().expect("board").midi_sources != count;
                    if changed {
                        on_sources.with(|b| b.midi_sources = count);
                    }
                }),
            );
            if let Err(e) = app.midi.set_output(midi_output.as_deref()) {
                app.with(|b| b.fail(format!("Sortie MIDI indisponible : {e}")));
            }
            if let Some(accelerator) = shortcut {
                if let Err(e) = handle.global_shortcut().register(accelerator.as_str()) {
                    app.with(|b| {
                        b.fail(format!(
                            "Raccourci global « {accelerator} » indisponible : {e}"
                        ))
                    });
                }
            }

            // Installers register the scheme; this also covers runs that were not installed.
            #[cfg(any(windows, target_os = "linux"))]
            if let Err(e) = handle.deep_link().register_all() {
                eprintln!("soundboard:// links not registered: {e}");
            }
            let links = app.clone();
            handle.deep_link().on_open_url(move |event| {
                let urls: Vec<String> = event.urls().iter().map(|u| u.to_string()).collect();
                links.with(|b| b.open(&urls));
            });
            let mut opened = set_files(std::env::args().skip(1));
            if let Ok(Some(urls)) = handle.deep_link().get_current() {
                opened.extend(urls.iter().map(|u| u.to_string()));
            }
            if !opened.is_empty() {
                app.with(|b| b.open(&opened));
            }
            update::check_in_background(handle);
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) = event {
                let paths = paths.clone();
                window.state::<Shared>().with(|b| b.drop_files(&paths));
            }
        })
        .invoke_handler(tauri::generate_handler![
            snapshot,
            dismiss_error,
            select_set,
            step_set,
            add_set,
            add_smart_set,
            rename_set,
            delete_set,
            set_rules,
            count_matching,
            set_display_mode,
            set_playback_mode,
            set_crossfade,
            reset_list,
            choose_sounds,
            import_sets,
            export_set,
            press,
            release,
            transport,
            hotkey,
            select_tile,
            update_tile,
            delete_tile,
            move_tile,
            set_hotkey,
            midi_learn,
            clear_midi,
            set_master,
            master_action,
            preview,
            stop_preview,
            cue,
            scan,
            jump,
            playhead,
            waveform,
            settings,
            set_output,
            set_preview,
            set_osc,
            set_midi_output,
            set_global_shortcut,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Soundboard");

    app.run(|handle, event| match event {
        RunEvent::Exit => handle
            .state::<Shared>()
            .board
            .lock()
            .expect("board")
            .flush_saves(),
        #[cfg(target_os = "macos")]
        RunEvent::Opened { urls } => {
            let files: Vec<String> = urls
                .iter()
                .filter_map(|u| u.to_file_path().ok())
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            if !files.is_empty() {
                handle.state::<Shared>().with(|b| b.open(&files));
            }
        }
        _ => {}
    });
}
