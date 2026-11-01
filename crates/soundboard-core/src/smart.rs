//! Smart sets: rules that pick tiles of ordinary sets.

use crate::model::{Tile, TileColor};
use crate::text::fold;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Match {
    /// Every rule must match.
    #[default]
    All,
    /// At least one rule must match.
    Any,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextComparison {
    Contains,
    BeginsWith,
    EndsWith,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Flag {
    Favorite,
    Loops,
    HoldToPlay,
    Solo,
    Played,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum SmartRule {
    Title {
        comparison: TextComparison,
        text: String,
    },
    Color {
        color: TileColor,
    },
    Flag {
        flag: Flag,
        value: bool,
    },
}

impl SmartRule {
    pub fn matches(&self, tile: &Tile) -> bool {
        match self {
            SmartRule::Title { comparison, text } => {
                let text = fold(text.trim());
                if text.is_empty() {
                    return false;
                }
                let title = fold(&tile.title);
                match comparison {
                    TextComparison::Contains => title.contains(&text),
                    TextComparison::BeginsWith => title.starts_with(&text),
                    TextComparison::EndsWith => title.ends_with(&text),
                }
            }
            SmartRule::Color { color } => tile.color == *color,
            SmartRule::Flag { flag, value } => {
                let actual = match flag {
                    Flag::Favorite => tile.is_favorite,
                    Flag::Loops => tile.player.loops,
                    Flag::HoldToPlay => tile.hold_to_play,
                    Flag::Solo => tile.solo,
                    Flag::Played => tile.last_played.is_some(),
                };
                actual == *value
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SmartRules {
    pub r#match: Match,
    pub rules: Vec<SmartRule>,
}

impl Default for SmartRules {
    /// A new smart set shows favourite tiles.
    fn default() -> Self {
        SmartRules {
            r#match: Match::All,
            rules: vec![SmartRule::Flag {
                flag: Flag::Favorite,
                value: true,
            }],
        }
    }
}

impl SmartRules {
    /// A smart set without rules matches nothing, rather than every tile.
    pub fn matches(&self, tile: &Tile) -> bool {
        if self.rules.is_empty() {
            return false;
        }
        match self.r#match {
            Match::All => self.rules.iter().all(|r| r.matches(tile)),
            Match::Any => self.rules.iter().any(|r| r.matches(tile)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::tests::tile;

    fn title(comparison: TextComparison, text: &str) -> SmartRule {
        SmartRule::Title {
            comparison,
            text: text.into(),
        }
    }

    #[test]
    fn titles_match_ignoring_case_and_accents() {
        let t = tile("Applaudissements Énormes");
        assert!(title(TextComparison::Contains, "enorme").matches(&t));
        assert!(title(TextComparison::BeginsWith, "APPLAU").matches(&t));
        assert!(title(TextComparison::EndsWith, "énormes").matches(&t));
        assert!(!title(TextComparison::BeginsWith, "énormes").matches(&t));
        assert!(
            !title(TextComparison::Contains, "  ").matches(&t),
            "blank text matches nothing"
        );
    }

    #[test]
    fn flags_and_colours() {
        let mut t = tile("x");
        t.is_favorite = true;
        t.color = TileColor::Green;
        assert!(SmartRules::default().matches(&t));
        assert!(SmartRule::Color {
            color: TileColor::Green
        }
        .matches(&t));
        let played = SmartRule::Flag {
            flag: Flag::Played,
            value: false,
        };
        assert!(played.matches(&t));
        t.last_played = Some(1.0);
        assert!(!played.matches(&t));
    }

    #[test]
    fn all_any_and_empty() {
        let mut t = tile("Pluie");
        t.is_favorite = true;
        let mut rules = SmartRules {
            r#match: Match::All,
            rules: vec![
                SmartRule::Flag {
                    flag: Flag::Favorite,
                    value: true,
                },
                SmartRule::Flag {
                    flag: Flag::Loops,
                    value: true,
                },
            ],
        };
        assert!(!rules.matches(&t));
        rules.r#match = Match::Any;
        assert!(rules.matches(&t));
        rules.rules.clear();
        assert!(!rules.matches(&t));
    }
}
