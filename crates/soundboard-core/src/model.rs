//! Sets of sound tiles, as saved in a set's `Set.json`.

use crate::midi::MidiNote;
use crate::smart::SmartRules;
use crate::{Seconds, Uuid};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// What happens when a tile finishes playing, in list mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlaybackMode {
    /// Nothing happens.
    Stop,
    /// The next playable tile is selected but not started.
    #[default]
    CueNext,
    /// The next playable tile starts automatically, crossfading into it.
    Continuous,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DisplayMode {
    #[default]
    Grid,
    List,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TileColor {
    #[default]
    Purple,
    Pink,
    Red,
    Orange,
    Yellow,
    Green,
    LightBlue,
    DarkBlue,
    Black,
}

impl TileColor {
    pub const ALL: [TileColor; 9] = [
        TileColor::Purple,
        TileColor::Pink,
        TileColor::Red,
        TileColor::Orange,
        TileColor::Yellow,
        TileColor::Green,
        TileColor::LightBlue,
        TileColor::DarkBlue,
        TileColor::Black,
    ];
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PlayerSettings {
    /// Trim, in seconds from the start and from the end of the file.
    pub trim_start: Seconds,
    pub trim_end: Seconds,
    pub fade_in_time: Seconds,
    pub fade_out_time: Seconds,
    pub loops: bool,
    pub volume_a: f32,
    pub volume_b: f32,
    /// When true the tile plays at `volume_b`.
    pub uses_volume_b: bool,
    pub muted: bool,
}

impl Default for PlayerSettings {
    fn default() -> Self {
        PlayerSettings {
            trim_start: 0.0,
            trim_end: 0.0,
            fade_in_time: 0.0,
            fade_out_time: 0.0,
            loops: false,
            volume_a: 1.0,
            volume_b: 0.4,
            uses_volume_b: false,
            muted: false,
        }
    }
}

impl PlayerSettings {
    pub fn active_volume(&self) -> f32 {
        if self.uses_volume_b {
            self.volume_b
        } else {
            self.volume_a
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GridPosition {
    pub column: i32,
    pub row: i32,
}

impl GridPosition {
    pub fn new(column: i32, row: i32) -> Self {
        GridPosition { column, row }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tile {
    pub id: Uuid,
    /// Audio file stored inside the set's folder.
    pub file_id: Uuid,
    pub original_file_name: String,
    pub title: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub color: TileColor,
    #[serde(default = "enabled")]
    pub is_enabled: bool,
    #[serde(default)]
    pub hold_to_play: bool,
    #[serde(default)]
    pub solo: bool,
    #[serde(default)]
    pub is_favorite: bool,
    /// Unix time in seconds.
    #[serde(default)]
    pub last_played: Option<f64>,
    /// Key that plays the tile while its set is shown; see `hotkey::normalized`.
    #[serde(default)]
    pub hotkey: Option<String>,
    /// Controller note that plays the tile while its set is shown.
    #[serde(default)]
    pub midi_trigger: Option<MidiNote>,
    #[serde(default)]
    pub grid_position: GridPosition,
    #[serde(default)]
    pub list_index: i32,
    #[serde(default)]
    pub player: PlayerSettings,
}

fn enabled() -> bool {
    true
}

impl Tile {
    pub fn new(file_id: Uuid, original_file_name: &str, title: &str) -> Self {
        Tile {
            id: Uuid::new_v4(),
            file_id,
            original_file_name: original_file_name.to_owned(),
            title: title.to_owned(),
            icon: String::new(),
            notes: String::new(),
            color: TileColor::Purple,
            is_enabled: true,
            hold_to_play: false,
            solo: false,
            is_favorite: false,
            last_played: None,
            hotkey: None,
            midi_trigger: None,
            grid_position: GridPosition::default(),
            list_index: 0,
            player: PlayerSettings::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SoundSet {
    pub format_version: u32,
    pub id: Uuid,
    pub title: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub display_mode: DisplayMode,
    #[serde(default)]
    pub playback_mode: PlaybackMode,
    #[serde(default)]
    pub crossfade_duration: Seconds,
    #[serde(default)]
    pub tiles: Vec<Tile>,
    /// Set for smart sets, which show matching tiles of ordinary sets instead of their own.
    #[serde(default)]
    pub smart_rules: Option<SmartRules>,
}

impl SoundSet {
    /// Bumped whenever the on-disk shape changes incompatibly.
    pub const FORMAT_VERSION: u32 = 1;

    pub fn new(title: &str) -> Self {
        SoundSet {
            format_version: Self::FORMAT_VERSION,
            id: Uuid::new_v4(),
            title: title.to_owned(),
            notes: String::new(),
            display_mode: DisplayMode::Grid,
            playback_mode: PlaybackMode::CueNext,
            crossfade_duration: 0.0,
            tiles: Vec::new(),
            smart_rules: None,
        }
    }

    pub fn is_smart(&self) -> bool {
        self.smart_rules.is_some()
    }

    /// Tiles in the grid's reading order.
    pub fn grid_order(&self) -> Vec<&Tile> {
        let mut tiles: Vec<&Tile> = self.tiles.iter().collect();
        tiles.sort_by_key(|t| (t.grid_position.row, t.grid_position.column));
        tiles
    }

    /// Tiles in list order; equal indices (tiles added before list order was kept) follow the grid.
    pub fn list_order(&self) -> Vec<&Tile> {
        let mut tiles: Vec<&Tile> = self.tiles.iter().collect();
        tiles.sort_by_key(|t| (t.list_index, t.grid_position.row, t.grid_position.column));
        tiles
    }

    pub fn display_order(&self) -> Vec<&Tile> {
        match self.display_mode {
            DisplayMode::List => self.list_order(),
            DisplayMode::Grid => self.grid_order(),
        }
    }

    /// List index for a tile added at the end of the list.
    pub fn next_list_index(&self) -> i32 {
        self.tiles.iter().map(|t| t.list_index).max().unwrap_or(-1) + 1
    }

    pub fn tile(&self, id: Uuid) -> Option<&Tile> {
        self.tiles.iter().find(|t| t.id == id)
    }

    pub fn first_playable_tile(&self) -> Option<&Tile> {
        self.display_order().into_iter().find(|t| t.is_enabled)
    }

    /// The next enabled tile after `tile_id` in display order; the list does not wrap around.
    pub fn next_playable_tile(&self, tile_id: Uuid) -> Option<&Tile> {
        let order = self.display_order();
        let index = order.iter().position(|t| t.id == tile_id)?;
        order[index + 1..].iter().copied().find(|t| t.is_enabled)
    }

    /// The crossfade setting while Continuous is active, `None` otherwise.
    /// Advance modes only act in list mode; a grid never advances on its own.
    pub fn continuous_crossfade(&self) -> Option<Seconds> {
        (self.display_mode == DisplayMode::List && self.playback_mode == PlaybackMode::Continuous)
            .then_some(self.crossfade_duration)
    }

    /// Tile to select after `tile_id` stops, in Cue next mode.
    pub fn tile_to_cue(&self, after_stopping: Uuid) -> Option<&Tile> {
        if self.display_mode != DisplayMode::List || self.playback_mode != PlaybackMode::CueNext {
            return None;
        }
        self.next_playable_tile(after_stopping)
    }

    /// How long before the end of a sound Continuous starts the next one and crossfades:
    /// the set's crossfade, but never more than half the sound.
    pub fn crossfade_lead(crossfade: Seconds, sound_length: Seconds) -> Seconds {
        crossfade.max(0.0).min(sound_length.max(0.0) / 2.0)
    }

    /// The same set under a new set id and new tile ids. Sound file ids are kept because
    /// they only name files inside the set's own folder.
    pub fn with_fresh_identifiers(&self) -> SoundSet {
        let mut copy = self.clone();
        copy.id = Uuid::new_v4();
        for tile in &mut copy.tiles {
            tile.id = Uuid::new_v4();
        }
        copy
    }

    /// Moves a tile to `index` in the current display order and renumbers that order only:
    /// list moves rewrite list indices, grid moves rewrite grid cells (packing any gaps).
    pub fn move_tile(&mut self, tile_id: Uuid, index: usize, grid_columns: i32) {
        let mut order: Vec<Uuid> = self.display_order().iter().map(|t| t.id).collect();
        let Some(from) = order.iter().position(|&id| id == tile_id) else {
            return;
        };
        let moved = order.remove(from);
        order.insert(index.min(order.len()), moved);
        let positions: HashMap<Uuid, i32> = order
            .iter()
            .enumerate()
            .map(|(i, &id)| (id, i as i32))
            .collect();
        let mode = self.display_mode;
        for tile in &mut self.tiles {
            let Some(&position) = positions.get(&tile.id) else {
                continue;
            };
            match mode {
                DisplayMode::List => tile.list_index = position,
                DisplayMode::Grid => {
                    tile.grid_position =
                        GridPosition::new(position % grid_columns, position / grid_columns)
                }
            }
        }
    }

    /// First empty cell in reading order, so deleted tiles leave a gap that new tiles fill.
    pub fn next_free_grid_position(&self, columns: i32) -> GridPosition {
        let taken: HashSet<GridPosition> = self.tiles.iter().map(|t| t.grid_position).collect();
        (0..)
            .map(|i| GridPosition::new(i % columns, i / columns))
            .find(|p| !taken.contains(p))
            .expect("a grid has unbounded cells")
    }

    pub fn tile_at(&self, position: GridPosition) -> Option<&Tile> {
        self.tiles.iter().find(|t| t.grid_position == position)
    }

    pub fn tile_for_hotkey(&self, key: &str) -> Option<&Tile> {
        let key = crate::hotkey::normalized(key)?;
        self.tiles
            .iter()
            .find(|t| t.hotkey.as_deref() == Some(key.as_str()))
    }

    /// Gives `key` to one tile, taking it away from any other tile of the set;
    /// `None` or an invalid key clears the tile's hotkey.
    pub fn assign_hotkey(&mut self, key: Option<&str>, tile_id: Uuid) {
        let key = key.and_then(crate::hotkey::normalized);
        for tile in &mut self.tiles {
            if tile.id == tile_id {
                tile.hotkey = key.clone();
            } else if key.is_some() && tile.hotkey == key {
                tile.hotkey = None;
            }
        }
    }

    pub fn tile_for_midi(&self, note: MidiNote) -> Option<&Tile> {
        self.tiles.iter().find(|t| t.midi_trigger == Some(note))
    }

    /// Gives `note` to one tile, taking it away from any other tile of the set; `None` clears it.
    pub fn assign_midi(&mut self, note: Option<MidiNote>, tile_id: Uuid) {
        for tile in &mut self.tiles {
            if tile.id == tile_id {
                tile.midi_trigger = note;
            } else if note.is_some() && tile.midi_trigger == note {
                tile.midi_trigger = None;
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn tile(title: &str) -> Tile {
        Tile::new(Uuid::new_v4(), &format!("{title}.wav"), title)
    }

    /// A list-mode set of `count` tiles, in list order.
    pub(crate) fn list_set(count: usize) -> SoundSet {
        let mut set = SoundSet::new("Liste");
        set.display_mode = DisplayMode::List;
        for i in 0..count {
            let mut t = tile(&format!("T{i}"));
            t.list_index = i as i32;
            t.grid_position = GridPosition::new(i as i32 % 6, i as i32 / 6);
            set.tiles.push(t);
        }
        set
    }

    fn ids(tiles: &[&Tile]) -> Vec<Uuid> {
        tiles.iter().map(|t| t.id).collect()
    }

    #[test]
    fn grid_order_reads_rows_then_columns() {
        let mut set = SoundSet::new("Grille");
        for (c, r) in [(1, 1), (0, 1), (2, 0)] {
            let mut t = tile("x");
            t.grid_position = GridPosition::new(c, r);
            set.tiles.push(t);
        }
        let order: Vec<_> = set.grid_order().iter().map(|t| t.grid_position).collect();
        assert_eq!(
            order,
            [
                GridPosition::new(2, 0),
                GridPosition::new(0, 1),
                GridPosition::new(1, 1)
            ]
        );
    }

    #[test]
    fn next_playable_skips_disabled_and_does_not_wrap() {
        let mut set = list_set(4);
        set.tiles[1].is_enabled = false;
        let order = ids(&set.list_order());
        assert_eq!(
            set.next_playable_tile(order[0]).map(|t| t.id),
            Some(order[2])
        );
        assert_eq!(set.next_playable_tile(order[3]).map(|t| t.id), None);
    }

    #[test]
    fn advance_modes_only_act_in_list_mode() {
        let mut set = list_set(2);
        set.playback_mode = PlaybackMode::Continuous;
        set.crossfade_duration = 1.5;
        assert_eq!(set.continuous_crossfade(), Some(1.5));
        set.display_mode = DisplayMode::Grid;
        assert_eq!(set.continuous_crossfade(), None);
        set.playback_mode = PlaybackMode::CueNext;
        assert!(set.tile_to_cue(set.tiles[0].id).is_none());
        set.display_mode = DisplayMode::List;
        assert_eq!(
            set.tile_to_cue(set.tiles[0].id).map(|t| t.id),
            Some(set.tiles[1].id)
        );
    }

    #[test]
    fn crossfade_lead_is_at_most_half_the_sound() {
        assert_eq!(SoundSet::crossfade_lead(3.0, 4.0), 2.0);
        assert_eq!(SoundSet::crossfade_lead(1.0, 4.0), 1.0);
        assert_eq!(SoundSet::crossfade_lead(-1.0, 4.0), 0.0);
    }

    #[test]
    fn moving_in_a_list_renumbers_list_indices_only() {
        let mut set = list_set(3);
        let before = ids(&set.list_order());
        let grid = set.tiles[0].grid_position;
        set.move_tile(before[0], 2, 6);
        assert_eq!(
            ids(&set.list_order()),
            vec![before[1], before[2], before[0]]
        );
        assert_eq!(set.tiles[0].grid_position, grid);
    }

    #[test]
    fn moving_in_a_grid_packs_cells() {
        let mut set = SoundSet::new("Grille");
        for c in [0, 3, 5] {
            let mut t = tile("x");
            t.grid_position = GridPosition::new(c, 0);
            set.tiles.push(t);
        }
        let first = set.tiles[0].id;
        set.move_tile(first, 2, 6);
        let cells: Vec<_> = set
            .grid_order()
            .iter()
            .map(|t| t.grid_position.column)
            .collect();
        assert_eq!(cells, [0, 1, 2]);
        assert_eq!(
            set.tile(first).unwrap().grid_position,
            GridPosition::new(2, 0)
        );
    }

    #[test]
    fn new_tiles_fill_the_first_gap() {
        let mut set = SoundSet::new("Grille");
        for c in [0, 2] {
            let mut t = tile("x");
            t.grid_position = GridPosition::new(c, 0);
            set.tiles.push(t);
        }
        assert_eq!(set.next_free_grid_position(6), GridPosition::new(1, 0));
        assert_eq!(SoundSet::new("vide").next_list_index(), 0);
    }

    #[test]
    fn a_hotkey_belongs_to_one_tile() {
        let mut set = list_set(2);
        let (a, b) = (set.tiles[0].id, set.tiles[1].id);
        set.assign_hotkey(Some("Q"), a);
        set.assign_hotkey(Some("q"), b);
        assert_eq!(set.tile(a).unwrap().hotkey, None);
        assert_eq!(set.tile_for_hotkey("Q").map(|t| t.id), Some(b));
        set.assign_hotkey(Some("é"), b);
        assert_eq!(set.tile(b).unwrap().hotkey, None, "invalid keys clear");
    }

    #[test]
    fn fresh_identifiers_keep_sound_files() {
        let set = list_set(1);
        let copy = set.with_fresh_identifiers();
        assert_ne!(copy.id, set.id);
        assert_ne!(copy.tiles[0].id, set.tiles[0].id);
        assert_eq!(copy.tiles[0].file_id, set.tiles[0].file_id);
    }

    #[test]
    fn missing_fields_decode_to_defaults() {
        let json = format!(
            r#"{{"formatVersion":1,"id":"{}","title":"Vieux","tiles":[{{"id":"{}","fileId":"{}","originalFileName":"a.wav","title":"A"}}]}}"#,
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4()
        );
        let set: SoundSet = serde_json::from_str(&json).unwrap();
        assert!(set.tiles[0].is_enabled);
        assert_eq!(set.tiles[0].player.volume_b, 0.4);
        assert_eq!(set.playback_mode, PlaybackMode::CueNext);
    }
}
