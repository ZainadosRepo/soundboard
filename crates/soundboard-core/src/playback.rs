//! Decides what happens when tiles are triggered; the audio engine only executes the commands.

use crate::model::Tile;
use crate::{Seconds, Uuid};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum VoiceState {
    Playing,
    Paused,
}

/// Modifier held while triggering a tile.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TriggerModifier {
    #[default]
    None,
    /// Pause a playing tile instead of stopping it.
    Pause,
    /// Fade out instead of cutting: applies to the tile itself and to sounds a solo tile stops.
    Fade,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlaybackCommand {
    /// `fade_in` > 0 starts from silence and ramps up linearly; `velocity` (0…1) scales the
    /// sound's gain, 1 for mouse and keyboard triggers.
    Start {
        tile: Uuid,
        fade_in: Seconds,
        velocity: f32,
    },
    /// `fade == None` stops immediately.
    Stop {
        tile: Uuid,
        fade: Option<Seconds>,
    },
    Pause(Uuid),
    Resume(Uuid),
}

impl PlaybackCommand {
    pub fn tile_id(&self) -> Uuid {
        match *self {
            PlaybackCommand::Start { tile, .. } | PlaybackCommand::Stop { tile, .. } => tile,
            PlaybackCommand::Pause(tile) | PlaybackCommand::Resume(tile) => tile,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlaybackCoordinator {
    states: HashMap<Uuid, VoiceState>,
    /// Global fade time, used for faded stops and Fade Out All.
    pub fade_time: Seconds,
}

impl Default for PlaybackCoordinator {
    fn default() -> Self {
        PlaybackCoordinator {
            states: HashMap::new(),
            fade_time: 2.0,
        }
    }
}

impl PlaybackCoordinator {
    pub fn state(&self, tile_id: Uuid) -> Option<VoiceState> {
        self.states.get(&tile_id).copied()
    }

    pub fn states(&self) -> &HashMap<Uuid, VoiceState> {
        &self.states
    }

    /// Mouse-down / key-down on a tile. In list mode Solo and Hold to Play are ignored.
    pub fn press(
        &mut self,
        tile: &Tile,
        modifier: TriggerModifier,
        list_mode: bool,
        velocity: f32,
    ) -> Vec<PlaybackCommand> {
        if !tile.is_enabled {
            return vec![];
        }
        let solo = tile.solo && !list_mode;
        let hold = tile.hold_to_play && !list_mode;
        let id = tile.id;
        if hold {
            return match self.state(id) {
                None => self.start(id, solo, modifier, velocity),
                Some(_) => vec![],
            };
        }
        match (self.state(id), modifier) {
            (None, _) => self.start(id, solo, modifier, velocity),
            (Some(VoiceState::Playing), TriggerModifier::Pause) => {
                self.states.insert(id, VoiceState::Paused);
                vec![PlaybackCommand::Pause(id)]
            }
            (Some(VoiceState::Playing), TriggerModifier::Fade) => {
                self.states.remove(&id);
                vec![PlaybackCommand::Stop {
                    tile: id,
                    fade: Some(self.fade_time),
                }]
            }
            (Some(VoiceState::Playing), TriggerModifier::None) => {
                self.states.remove(&id);
                vec![PlaybackCommand::Stop {
                    tile: id,
                    fade: None,
                }]
            }
            (Some(VoiceState::Paused), _) => {
                self.states.insert(id, VoiceState::Playing);
                vec![PlaybackCommand::Resume(id)]
            }
        }
    }

    /// Mouse-up / key-up on a tile: only Hold to Play tiles react, and never in list mode.
    pub fn release(&mut self, tile: &Tile, list_mode: bool) -> Vec<PlaybackCommand> {
        if list_mode || !tile.hold_to_play || self.states.remove(&tile.id).is_none() {
            return vec![];
        }
        vec![PlaybackCommand::Stop {
            tile: tile.id,
            fade: None,
        }]
    }

    /// The engine reports that a voice ended on its own (end of file or faded to silence).
    pub fn voice_did_finish(&mut self, tile_id: Uuid) {
        self.states.remove(&tile_id);
    }

    /// Continuous mode: start `next` fading in over `duration` and fade the outgoing tile out
    /// over the same time, unless it loops, in which case it keeps playing underneath.
    pub fn crossfade(
        &mut self,
        outgoing: &Tile,
        next: &Tile,
        duration: Seconds,
    ) -> Vec<PlaybackCommand> {
        if !next.is_enabled || next.id == outgoing.id || self.state(next.id).is_some() {
            return vec![];
        }
        let mut commands = vec![];
        if !outgoing.player.loops && self.states.remove(&outgoing.id).is_some() {
            commands.push(PlaybackCommand::Stop {
                tile: outgoing.id,
                fade: (duration > 0.0).then_some(duration),
            });
        }
        self.states.insert(next.id, VoiceState::Playing);
        commands.push(PlaybackCommand::Start {
            tile: next.id,
            fade_in: duration,
            velocity: 1.0,
        });
        commands
    }

    /// Pauses every playing sound; when nothing plays, resumes every paused one.
    pub fn toggle_pause_all(&mut self) -> Vec<PlaybackCommand> {
        let playing = self.sorted(|s| s == VoiceState::Playing);
        let (affected, target) = if playing.is_empty() {
            (
                self.sorted(|s| s == VoiceState::Paused),
                VoiceState::Playing,
            )
        } else {
            (playing, VoiceState::Paused)
        };
        affected
            .into_iter()
            .map(|id| {
                self.states.insert(id, target);
                match target {
                    VoiceState::Paused => PlaybackCommand::Pause(id),
                    VoiceState::Playing => PlaybackCommand::Resume(id),
                }
            })
            .collect()
    }

    /// Stops one tile whatever its state (playing or paused); nothing if it is idle.
    pub fn stop(&mut self, tile_id: Uuid, fade: Option<Seconds>) -> Vec<PlaybackCommand> {
        match self.states.remove(&tile_id) {
            Some(_) => vec![PlaybackCommand::Stop {
                tile: tile_id,
                fade,
            }],
            None => vec![],
        }
    }

    pub fn stop_all(&mut self) -> Vec<PlaybackCommand> {
        self.stop_everything(None, None)
    }

    pub fn fade_out_all(&mut self) -> Vec<PlaybackCommand> {
        self.stop_everything(None, Some(self.fade_time))
    }

    fn start(
        &mut self,
        id: Uuid,
        solo: bool,
        modifier: TriggerModifier,
        velocity: f32,
    ) -> Vec<PlaybackCommand> {
        let mut commands = vec![];
        if solo {
            let fade = (modifier == TriggerModifier::Fade).then_some(self.fade_time);
            commands = self.stop_everything(Some(id), fade);
        }
        self.states.insert(id, VoiceState::Playing);
        commands.push(PlaybackCommand::Start {
            tile: id,
            fade_in: 0.0,
            velocity,
        });
        commands
    }

    fn stop_everything(
        &mut self,
        kept: Option<Uuid>,
        fade: Option<Seconds>,
    ) -> Vec<PlaybackCommand> {
        let stopped = self.sorted(|_| true);
        stopped
            .into_iter()
            .filter(|id| Some(*id) != kept)
            .map(|id| {
                self.states.remove(&id);
                PlaybackCommand::Stop { tile: id, fade }
            })
            .collect()
    }

    /// Ids in a fixed order so commands are deterministic.
    fn sorted(&self, keep: impl Fn(VoiceState) -> bool) -> Vec<Uuid> {
        let mut ids: Vec<Uuid> = self
            .states
            .iter()
            .filter(|(_, s)| keep(**s))
            .map(|(id, _)| *id)
            .collect();
        ids.sort();
        ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::tests::tile;
    use PlaybackCommand::*;

    fn start(id: Uuid) -> PlaybackCommand {
        Start {
            tile: id,
            fade_in: 0.0,
            velocity: 1.0,
        }
    }

    #[test]
    fn click_starts_then_stops() {
        let mut c = PlaybackCoordinator::default();
        let t = tile("a");
        assert_eq!(
            c.press(&t, TriggerModifier::None, false, 1.0),
            [start(t.id)]
        );
        assert_eq!(c.state(t.id), Some(VoiceState::Playing));
        assert_eq!(
            c.press(&t, TriggerModifier::None, false, 1.0),
            [Stop {
                tile: t.id,
                fade: None
            }]
        );
        assert_eq!(c.state(t.id), None);
    }

    #[test]
    fn shift_pauses_and_any_press_resumes() {
        let mut c = PlaybackCoordinator::default();
        let t = tile("a");
        c.press(&t, TriggerModifier::None, false, 1.0);
        assert_eq!(
            c.press(&t, TriggerModifier::Pause, false, 1.0),
            [Pause(t.id)]
        );
        assert_eq!(
            c.press(&t, TriggerModifier::Fade, false, 1.0),
            [Resume(t.id)]
        );
    }

    #[test]
    fn fade_modifier_fades_out_with_the_global_time() {
        let mut c = PlaybackCoordinator::default();
        let t = tile("a");
        c.press(&t, TriggerModifier::None, false, 1.0);
        assert_eq!(
            c.press(&t, TriggerModifier::Fade, false, 1.0),
            [Stop {
                tile: t.id,
                fade: Some(2.0)
            }]
        );
    }

    #[test]
    fn disabled_tiles_do_nothing() {
        let mut c = PlaybackCoordinator::default();
        let mut t = tile("a");
        t.is_enabled = false;
        assert!(c.press(&t, TriggerModifier::None, false, 1.0).is_empty());
    }

    #[test]
    fn solo_stops_the_others_unless_in_list_mode() {
        let mut c = PlaybackCoordinator::default();
        let other = tile("autre");
        let mut solo = tile("solo");
        solo.solo = true;
        c.press(&other, TriggerModifier::None, false, 1.0);
        assert_eq!(
            c.press(&solo, TriggerModifier::Fade, false, 1.0),
            [
                Stop {
                    tile: other.id,
                    fade: Some(2.0)
                },
                start(solo.id)
            ]
        );
        let mut c = PlaybackCoordinator::default();
        c.press(&other, TriggerModifier::None, true, 1.0);
        assert_eq!(
            c.press(&solo, TriggerModifier::None, true, 1.0),
            [start(solo.id)]
        );
    }

    #[test]
    fn hold_to_play_starts_on_press_and_stops_on_release() {
        let mut c = PlaybackCoordinator::default();
        let mut t = tile("a");
        t.hold_to_play = true;
        assert_eq!(
            c.press(&t, TriggerModifier::None, false, 1.0),
            [start(t.id)]
        );
        assert!(c.press(&t, TriggerModifier::None, false, 1.0).is_empty());
        assert_eq!(
            c.release(&t, false),
            [Stop {
                tile: t.id,
                fade: None
            }]
        );
        assert!(c.release(&t, false).is_empty());
        c.press(&t, TriggerModifier::None, true, 1.0);
        assert!(
            c.release(&t, true).is_empty(),
            "list mode ignores Hold to Play"
        );
    }

    #[test]
    fn velocity_reaches_the_start() {
        let mut c = PlaybackCoordinator::default();
        let t = tile("a");
        assert_eq!(
            c.press(&t, TriggerModifier::None, false, 0.5),
            [Start {
                tile: t.id,
                fade_in: 0.0,
                velocity: 0.5
            }]
        );
    }

    #[test]
    fn crossfade_fades_the_outgoing_tile_unless_it_loops() {
        let mut c = PlaybackCoordinator::default();
        let (a, b) = (tile("a"), tile("b"));
        c.press(&a, TriggerModifier::None, true, 1.0);
        assert_eq!(
            c.crossfade(&a, &b, 1.5),
            [
                Stop {
                    tile: a.id,
                    fade: Some(1.5)
                },
                Start {
                    tile: b.id,
                    fade_in: 1.5,
                    velocity: 1.0
                }
            ]
        );
        assert!(c.crossfade(&a, &b, 1.5).is_empty(), "next already plays");

        let mut c = PlaybackCoordinator::default();
        let mut looping = tile("loop");
        looping.player.loops = true;
        c.press(&looping, TriggerModifier::None, true, 1.0);
        assert_eq!(
            c.crossfade(&looping, &b, 0.0),
            [Start {
                tile: b.id,
                fade_in: 0.0,
                velocity: 1.0
            }]
        );
        assert_eq!(c.state(looping.id), Some(VoiceState::Playing));
    }

    #[test]
    fn pause_all_toggles() {
        let mut c = PlaybackCoordinator::default();
        let (a, b) = (tile("a"), tile("b"));
        c.press(&a, TriggerModifier::None, false, 1.0);
        c.press(&b, TriggerModifier::None, false, 1.0);
        assert_eq!(c.toggle_pause_all().len(), 2);
        assert!(c.states().values().all(|s| *s == VoiceState::Paused));
        let resumed = c.toggle_pause_all();
        assert!(resumed.iter().all(|cmd| matches!(cmd, Resume(_))));
    }

    #[test]
    fn stop_all_and_fade_out_all_clear_every_state() {
        let mut c = PlaybackCoordinator::default();
        let (a, b) = (tile("a"), tile("b"));
        c.press(&a, TriggerModifier::None, false, 1.0);
        c.press(&b, TriggerModifier::Pause, false, 1.0);
        c.press(&b, TriggerModifier::Pause, false, 1.0);
        assert!(c.fade_out_all().iter().all(|cmd| *cmd
            == Stop {
                tile: cmd.tile_id(),
                fade: Some(2.0)
            }));
        assert!(c.states().is_empty());
        assert!(c.stop_all().is_empty());
        assert!(c.stop(a.id, None).is_empty());
    }
}
