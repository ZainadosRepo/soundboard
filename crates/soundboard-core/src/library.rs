//! All sound sets plus which one is shown.

use crate::model::{GridPosition, SoundSet, Tile};
use crate::text::natural_cmp;
use crate::Uuid;
use std::cmp::Ordering;

#[derive(Debug, Clone, PartialEq)]
pub struct SoundLibrary {
    sets: Vec<SoundSet>,
    selected: Uuid,
}

impl SoundLibrary {
    pub const DEFAULT_SET_TITLE: &'static str = "Set sans titre";

    /// Never empty: an empty library gets one untitled set. Sets are ordered by title.
    pub fn new(sets: Vec<SoundSet>, selected: Option<Uuid>) -> Self {
        let mut sets = if sets.is_empty() {
            vec![SoundSet::new(Self::DEFAULT_SET_TITLE)]
        } else {
            sets
        };
        sets.sort_by(order);
        let selected = selected
            .filter(|id| sets.iter().any(|s| s.id == *id))
            .unwrap_or(sets[0].id);
        SoundLibrary { sets, selected }
    }

    pub fn sets(&self) -> &[SoundSet] {
        &self.sets
    }

    pub fn selected_id(&self) -> Uuid {
        self.selected
    }

    pub fn selected(&self) -> &SoundSet {
        self.set(self.selected).unwrap_or(&self.sets[0])
    }

    pub fn set(&self, id: Uuid) -> Option<&SoundSet> {
        self.sets.iter().find(|s| s.id == id)
    }

    /// The set holding a tile, for playing tiles of sets that are not shown.
    pub fn set_containing(&self, tile_id: Uuid) -> Option<&SoundSet> {
        self.sets
            .iter()
            .find(|s| s.tiles.iter().any(|t| t.id == tile_id))
    }

    /// The tile as stored in its own set.
    pub fn tile(&self, tile_id: Uuid) -> Option<&Tile> {
        self.set_containing(tile_id)?.tile(tile_id)
    }

    pub fn select(&mut self, id: Uuid) {
        if self.set(id).is_some() {
            self.selected = id;
        }
    }

    /// Adds and selects a new empty set named "Set sans titre", "Set sans titre 2", …
    pub fn add_set(&mut self) -> SoundSet {
        let mut title = Self::DEFAULT_SET_TITLE.to_owned();
        let mut number = 2;
        while self.sets.iter().any(|s| s.title == title) {
            title = format!("{} {number}", Self::DEFAULT_SET_TITLE);
            number += 1;
        }
        let set = SoundSet::new(&title);
        self.add(set.clone());
        set
    }

    /// Adds an existing set, e.g. an imported one, and shows it.
    pub fn add(&mut self, set: SoundSet) {
        if self.set(set.id).is_some() {
            return;
        }
        self.selected = set.id;
        self.sets.push(set);
        self.sets.sort_by(order);
    }

    pub fn replace(&mut self, set: SoundSet) {
        if let Some(slot) = self.sets.iter_mut().find(|s| s.id == set.id) {
            *slot = set;
            self.sets.sort_by(order);
        }
    }

    /// Removes a set; the last remaining set cannot be removed.
    /// When the shown set goes, the one that took its place in the list is shown.
    pub fn remove_set(&mut self, id: Uuid) -> bool {
        let Some(index) = self.sets.iter().position(|s| s.id == id) else {
            return false;
        };
        if self.sets.len() < 2 {
            return false;
        }
        self.sets.remove(index);
        if self.selected == id {
            self.selected = self.sets[index.min(self.sets.len() - 1)].id;
        }
        true
    }

    pub fn select_next(&mut self) {
        self.move_selection(1);
    }

    pub fn select_previous(&mut self) {
        self.move_selection(-1);
    }

    fn move_selection(&mut self, offset: isize) {
        let Some(index) = self.sets.iter().position(|s| s.id == self.selected) else {
            return;
        };
        if let Some(set) = index
            .checked_add_signed(offset)
            .and_then(|i| self.sets.get(i))
        {
            self.selected = set.id;
        }
    }

    /// How many tiles a set shows, without laying them out.
    pub fn tile_count(&self, set: &SoundSet) -> usize {
        match &set.smart_rules {
            None => set.tiles.len(),
            Some(rules) => self
                .sets
                .iter()
                .filter(|s| !s.is_smart())
                .map(|s| s.tiles.iter().filter(|t| rules.matches(t)).count())
                .sum(),
        }
    }

    /// What a set shows: its own tiles, or for a smart set the matching tiles of the ordinary
    /// sets, in sidebar order then each set's grid order, laid out in a fresh grid.
    pub fn shown_set(&self, set: &SoundSet, grid_columns: i32) -> SoundSet {
        let Some(rules) = &set.smart_rules else {
            return set.clone();
        };
        let mut shown = set.clone();
        shown.tiles = self
            .sets
            .iter()
            .filter(|s| !s.is_smart())
            .flat_map(|s| s.grid_order().into_iter().filter(|t| rules.matches(t)))
            .enumerate()
            .map(|(i, t)| {
                let mut placed = t.clone();
                let i = i as i32;
                placed.grid_position = GridPosition::new(i % grid_columns, i / grid_columns);
                placed.list_index = i;
                placed
            })
            .collect();
        shown
    }
}

fn order(a: &SoundSet, b: &SoundSet) -> Ordering {
    natural_cmp(&a.title, &b.title).then_with(|| a.id.cmp(&b.id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::tests::tile;
    use crate::smart::SmartRules;

    fn titled(title: &str) -> SoundSet {
        SoundSet::new(title)
    }

    #[test]
    fn empty_library_gets_one_set() {
        let library = SoundLibrary::new(vec![], None);
        assert_eq!(library.sets().len(), 1);
        assert_eq!(library.selected().title, SoundLibrary::DEFAULT_SET_TITLE);
    }

    #[test]
    fn sets_are_sorted_and_unknown_selection_falls_back() {
        let library = SoundLibrary::new(
            vec![titled("Set 10"), titled("set 2")],
            Some(Uuid::new_v4()),
        );
        let titles: Vec<_> = library.sets().iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, ["set 2", "Set 10"]);
        assert_eq!(library.selected_id(), library.sets()[0].id);
    }

    #[test]
    fn new_sets_get_numbered_titles() {
        let mut library = SoundLibrary::new(vec![], None);
        let second = library.add_set();
        assert_eq!(second.title, "Set sans titre 2");
        assert_eq!(library.selected_id(), second.id);
    }

    #[test]
    fn removing_the_shown_set_shows_its_successor_but_never_the_last() {
        let (a, b, c) = (titled("A"), titled("B"), titled("C"));
        let (ida, idb, idc) = (a.id, b.id, c.id);
        let mut library = SoundLibrary::new(vec![a, b, c], Some(idb));
        assert!(library.remove_set(idb));
        assert_eq!(library.selected_id(), idc);
        assert!(library.remove_set(idc));
        assert_eq!(library.selected_id(), ida);
        assert!(!library.remove_set(ida));
    }

    #[test]
    fn selection_moves_without_wrapping() {
        let (a, b) = (titled("A"), titled("B"));
        let (ida, idb) = (a.id, b.id);
        let mut library = SoundLibrary::new(vec![a, b], Some(ida));
        library.select_previous();
        assert_eq!(library.selected_id(), ida);
        library.select_next();
        library.select_next();
        assert_eq!(library.selected_id(), idb);
    }

    #[test]
    fn smart_sets_show_matching_tiles_of_ordinary_sets() {
        let mut a = titled("A");
        let mut fav = tile("Fav");
        fav.is_favorite = true;
        fav.grid_position = GridPosition::new(4, 2);
        a.tiles.push(fav.clone());
        a.tiles.push(tile("Autre"));
        let mut smart = titled("Smart");
        smart.smart_rules = Some(SmartRules::default());
        let mut nested = tile("Dans le smart");
        nested.is_favorite = true;
        smart.tiles.push(nested);
        let library = SoundLibrary::new(vec![a, smart.clone()], None);
        let shown = library.shown_set(&smart, 6);
        assert_eq!(shown.tiles.len(), 1);
        assert_eq!(shown.tiles[0].id, fav.id);
        assert_eq!(shown.tiles[0].grid_position, GridPosition::new(0, 0));
        assert_eq!(library.tile_count(&smart), 1);
    }
}
