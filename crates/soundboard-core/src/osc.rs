//! OSC 1.0 messages over UDP and what they ask the board to do.

use crate::model::GridPosition;

/// One argument of an OSC message (OSC 1.0 types plus the common T/F booleans).
#[derive(Debug, Clone, PartialEq)]
pub enum OscArgument {
    Int(i32),
    Float(f32),
    String(String),
    Bool(bool),
}

impl OscArgument {
    /// Numeric reading used by controls: buttons send 1/0, faders 0…1.
    pub fn number(&self) -> Option<f64> {
        match self {
            OscArgument::Int(v) => Some(f64::from(*v)),
            OscArgument::Float(v) => Some(f64::from(*v)),
            OscArgument::Bool(v) => Some(if *v { 1.0 } else { 0.0 }),
            OscArgument::String(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OscMessage {
    pub address: String,
    pub arguments: Vec<OscArgument>,
}

impl OscMessage {
    pub fn new(address: impl Into<String>, arguments: Vec<OscArgument>) -> Self {
        OscMessage {
            address: address.into(),
            arguments,
        }
    }

    /// Messages in a UDP datagram: a single message, or a bundle (possibly nested).
    /// Malformed data gives no messages rather than partial ones.
    pub fn decode_packet(data: &[u8]) -> Vec<OscMessage> {
        let mut reader = Reader {
            bytes: data,
            offset: 0,
        };
        reader.packet(data.len(), 0).unwrap_or_default()
    }

    /// The message as an OSC datagram.
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = padded(&self.address);
        let mut tags = String::from(",");
        let mut payload = Vec::new();
        for argument in &self.arguments {
            match argument {
                OscArgument::Int(v) => {
                    tags.push('i');
                    payload.extend_from_slice(&v.to_be_bytes());
                }
                OscArgument::Float(v) => {
                    tags.push('f');
                    payload.extend_from_slice(&v.to_bits().to_be_bytes());
                }
                OscArgument::String(v) => {
                    tags.push('s');
                    payload.extend(padded(v));
                }
                OscArgument::Bool(v) => tags.push(if *v { 'T' } else { 'F' }),
            }
        }
        bytes.extend(padded(&tags));
        bytes.extend(payload);
        bytes
    }
}

/// OSC strings are NUL-terminated and padded to a multiple of 4 bytes.
fn padded(s: &str) -> Vec<u8> {
    let mut bytes = s.as_bytes().to_vec();
    bytes.push(0);
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0);
    }
    bytes
}

struct Malformed;

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl Reader<'_> {
    /// Bundles nest; real senders use one or two levels, so deep nesting is refused.
    const MAXIMUM_BUNDLE_DEPTH: usize = 8;

    fn packet(&mut self, length: usize, depth: usize) -> Result<Vec<OscMessage>, Malformed> {
        let end = self.offset + length;
        if end > self.bytes.len() || depth > Self::MAXIMUM_BUNDLE_DEPTH {
            return Err(Malformed);
        }
        if self.bytes[self.offset..].starts_with(b"#bundle\0") {
            // The 8-byte time tag is ignored: messages act on arrival.
            self.offset += 16;
            let mut messages = Vec::new();
            while self.offset < end {
                let size = self.int32()?;
                if size <= 0 || size % 4 != 0 {
                    return Err(Malformed);
                }
                messages.extend(self.packet(size as usize, depth + 1)?);
            }
            return Ok(messages);
        }
        let message = self.message()?;
        if self.offset > end {
            return Err(Malformed);
        }
        self.offset = end;
        Ok(vec![message])
    }

    fn message(&mut self) -> Result<OscMessage, Malformed> {
        let address = self.string()?;
        if !address.starts_with('/') {
            return Err(Malformed);
        }
        // A message without a type tag string has no arguments (old OSC senders).
        if self.bytes.get(self.offset) != Some(&b',') {
            return Ok(OscMessage::new(address, vec![]));
        }
        let tags = self.string()?;
        let mut arguments = Vec::new();
        for tag in tags.chars().skip(1) {
            arguments.push(match tag {
                'i' => OscArgument::Int(self.int32()?),
                'f' => OscArgument::Float(f32::from_bits(self.int32()? as u32)),
                's' => OscArgument::String(self.string()?),
                'T' => OscArgument::Bool(true),
                'F' => OscArgument::Bool(false),
                // Unknown types have unknown sizes.
                _ => return Err(Malformed),
            });
        }
        Ok(OscMessage::new(address, arguments))
    }

    fn int32(&mut self) -> Result<i32, Malformed> {
        let word = self
            .bytes
            .get(self.offset..self.offset + 4)
            .ok_or(Malformed)?;
        self.offset += 4;
        Ok(i32::from_be_bytes(word.try_into().expect("four bytes")))
    }

    fn string(&mut self) -> Result<String, Malformed> {
        let rest = self.bytes.get(self.offset..).ok_or(Malformed)?;
        let terminator = self.offset + rest.iter().position(|&b| b == 0).ok_or(Malformed)?;
        let s = String::from_utf8_lossy(&self.bytes[self.offset..terminator]).into_owned();
        self.offset = (terminator + 4) & !3;
        if self.offset > self.bytes.len() {
            return Err(Malformed);
        }
        Ok(s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetReference {
    /// Position in the sidebar, from 0.
    Index(usize),
    /// The set shown in the window.
    Selected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileAction {
    Play,
    Fade,
    Loop,
    Mute,
    Solo,
    ToggleAb,
}

impl TileAction {
    fn parse(s: &str) -> Option<TileAction> {
        Some(match s {
            "play" => TileAction::Play,
            "fade" => TileAction::Fade,
            "loop" => TileAction::Loop,
            "mute" => TileAction::Mute,
            "solo" => TileAction::Solo,
            "toggleAB" => TileAction::ToggleAb,
            _ => return None,
        })
    }
}

/// What an OSC message (or a `soundboard://` link) asks the board to do. Addresses:
/// `/set/<n|selected>/tile/<column>/<row>/<action>`, `/master/…`, `/transport/…`,
/// `/global/bringForward`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OscCommand {
    /// `pressed` is false for the release of a button (value 0), which only matters to `Play`
    /// on Hold to Play tiles; other actions ignore releases.
    Tile {
        set: SetReference,
        position: GridPosition,
        action: TileAction,
        pressed: bool,
    },
    MasterToggleAb,
    MasterSelectVolumeA,
    MasterSelectVolumeB,
    MasterVolumeUp,
    MasterVolumeDown,
    /// 0…1, clamped.
    MasterVolume(f32),
    PlayPauseAll,
    StopAll,
    /// Brings the app forward, like the global shortcut.
    BringForward,
}

impl OscCommand {
    pub fn parse(message: &OscMessage) -> Option<OscCommand> {
        let path: Vec<&str> = message
            .address
            .split('/')
            .filter(|p| !p.is_empty())
            .collect();
        let value = message.arguments.first().and_then(OscArgument::number);
        // Buttons send 1 when pressed and 0 when released; a message without a value is a press.
        let pressed = value.unwrap_or(1.0) != 0.0;

        if let ["set", set, "tile", column, row, action] = path[..] {
            let set = match set {
                "selected" => SetReference::Selected,
                n => SetReference::Index(n.parse().ok()?),
            };
            let position = GridPosition::new(
                column.parse().ok().filter(|c: &i32| *c >= 0)?,
                row.parse().ok().filter(|r: &i32| *r >= 0)?,
            );
            return Some(OscCommand::Tile {
                set,
                position,
                action: TileAction::parse(action)?,
                pressed,
            });
        }
        let command = match path.join("/").as_str() {
            "master/volume" => OscCommand::MasterVolume(value?.clamp(0.0, 1.0) as f32),
            _ if !pressed => return None,
            "master/toggleAB" => OscCommand::MasterToggleAb,
            "master/selectVolumeA" => OscCommand::MasterSelectVolumeA,
            "master/selectVolumeB" => OscCommand::MasterSelectVolumeB,
            "master/volumeUp" => OscCommand::MasterVolumeUp,
            "master/volumeDown" => OscCommand::MasterVolumeDown,
            "transport/playPauseAll" => OscCommand::PlayPauseAll,
            "transport/stopAll" => OscCommand::StopAll,
            "global/bringForward" => OscCommand::BringForward,
            _ => return None,
        };
        Some(command)
    }

    /// Feedback telling a controller whether a tile plays, e.g. to light its button.
    pub fn playing_feedback(
        set_index: usize,
        position: GridPosition,
        is_playing: bool,
    ) -> OscMessage {
        OscMessage::new(
            format!(
                "/set/{set_index}/tile/{}/{}/play",
                position.column, position.row
            ),
            vec![OscArgument::Float(if is_playing { 1.0 } else { 0.0 })],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(address: &str, arguments: Vec<OscArgument>) -> OscMessage {
        OscMessage::new(address, arguments)
    }

    #[test]
    fn round_trips_every_argument_type() {
        let m = message(
            "/a/b",
            vec![
                OscArgument::Int(-3),
                OscArgument::Float(0.5),
                OscArgument::String("salut".into()),
                OscArgument::Bool(true),
                OscArgument::Bool(false),
            ],
        );
        let bytes = m.encode();
        assert_eq!(bytes.len() % 4, 0);
        assert_eq!(OscMessage::decode_packet(&bytes), [m]);
    }

    #[test]
    fn decodes_nested_bundles() {
        let inner = message("/transport/stopAll", vec![]).encode();
        let mut bundle = b"#bundle\0".to_vec();
        bundle.extend([0u8; 8]);
        bundle.extend((inner.len() as i32).to_be_bytes());
        bundle.extend(&inner);
        let mut outer = b"#bundle\0".to_vec();
        outer.extend([0u8; 8]);
        outer.extend((bundle.len() as i32).to_be_bytes());
        outer.extend(&bundle);
        assert_eq!(OscMessage::decode_packet(&outer).len(), 1);
    }

    #[test]
    fn malformed_data_gives_nothing() {
        assert!(OscMessage::decode_packet(b"/no-terminator").is_empty());
        assert!(OscMessage::decode_packet(b"nope\0\0\0\0").is_empty());
        let mut unknown = padded("/x");
        unknown.extend(padded(",b"));
        assert!(OscMessage::decode_packet(&unknown).is_empty());
        let mut bad_bundle = b"#bundle\0".to_vec();
        bad_bundle.extend([0u8; 8]);
        bad_bundle.extend(3i32.to_be_bytes());
        assert!(OscMessage::decode_packet(&bad_bundle).is_empty());
    }

    #[test]
    fn messages_without_type_tags_have_no_arguments() {
        assert_eq!(
            OscMessage::decode_packet(&padded("/transport/stopAll")),
            [message("/transport/stopAll", vec![])]
        );
    }

    #[test]
    fn tile_addresses() {
        assert_eq!(
            OscCommand::parse(&message(
                "/set/2/tile/3/1/play",
                vec![OscArgument::Float(0.0)]
            )),
            Some(OscCommand::Tile {
                set: SetReference::Index(2),
                position: GridPosition::new(3, 1),
                action: TileAction::Play,
                pressed: false,
            })
        );
        assert!(matches!(
            OscCommand::parse(&message("/set/selected/tile/0/0/toggleAB", vec![])),
            Some(OscCommand::Tile {
                set: SetReference::Selected,
                action: TileAction::ToggleAb,
                pressed: true,
                ..
            })
        ));
        assert_eq!(
            OscCommand::parse(&message("/set/-1/tile/0/0/play", vec![])),
            None
        );
        assert_eq!(
            OscCommand::parse(&message("/set/0/tile/0/0/dance", vec![])),
            None
        );
    }

    #[test]
    fn master_and_transport() {
        assert_eq!(
            OscCommand::parse(&message("/master/volume", vec![OscArgument::Float(1.5)])),
            Some(OscCommand::MasterVolume(1.0))
        );
        assert_eq!(OscCommand::parse(&message("/master/volume", vec![])), None);
        assert_eq!(
            OscCommand::parse(&message("/transport/stopAll", vec![OscArgument::Int(1)])),
            Some(OscCommand::StopAll)
        );
        assert_eq!(
            OscCommand::parse(&message("/transport/stopAll", vec![OscArgument::Int(0)])),
            None,
            "button releases are ignored"
        );
    }

    #[test]
    fn feedback_address() {
        let m = OscCommand::playing_feedback(1, GridPosition::new(2, 3), true);
        assert_eq!(m.address, "/set/1/tile/2/3/play");
        assert_eq!(m.arguments, [OscArgument::Float(1.0)]);
    }
}
