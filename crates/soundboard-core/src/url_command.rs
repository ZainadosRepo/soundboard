//! `soundboard://` links, e.g. from a Stream Deck, a script or a shortcut.
//!
//! - Tiles and sets by name: `soundboard://play?title=Bonjour` (also `stop`, `fade`;
//!   `&set=Effets` restricts the search to one set), `soundboard://set?title=Effets` shows a set.
//! - Every OSC address works as a path: `soundboard://transport/stopAll`,
//!   `soundboard://master/volume?value=0.5`, `soundboard://set/0/tile/1/0/play`.

use crate::osc::{OscArgument, OscCommand, OscMessage};
use std::collections::HashMap;

pub const SCHEME: &str = "soundboard";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkTileAction {
    Play,
    Stop,
    Fade,
}

#[derive(Debug, Clone, PartialEq)]
pub enum UrlCommand {
    Tile {
        title: String,
        set: Option<String>,
        action: LinkTileAction,
    },
    ShowSet {
        title: String,
    },
    Osc(OscCommand),
}

impl UrlCommand {
    pub fn parse(url: &str) -> Option<UrlCommand> {
        let (scheme, rest) = url.trim().split_once("://")?;
        if !scheme.eq_ignore_ascii_case(SCHEME) {
            return None;
        }
        let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
        let path = path.split('#').next().unwrap_or_default();
        let mut parameters: HashMap<String, String> = HashMap::new();
        for pair in query.split('#').next().unwrap_or_default().split('&') {
            let Some((name, value)) = pair.split_once('=') else {
                continue;
            };
            // Decoded once more: a link typed with unencoded accents gets encoded twice by some
            // senders (`%20` arrives as `%2520`), which would leave `%20` in a name.
            let value = percent_decode(&percent_decode(value));
            parameters.entry(percent_decode(name)).or_insert(value);
        }
        let path: Vec<String> = path
            .split('/')
            .filter(|p| !p.is_empty())
            .map(percent_decode)
            .collect();
        let title = parameters.get("title").filter(|t| !t.is_empty()).cloned();

        if let [action] = &path[..] {
            let action = match action.as_str() {
                "play" => Some(LinkTileAction::Play),
                "stop" => Some(LinkTileAction::Stop),
                "fade" => Some(LinkTileAction::Fade),
                _ => None,
            };
            if let Some(action) = action {
                return Some(UrlCommand::Tile {
                    title: title?,
                    set: parameters.get("set").cloned(),
                    action,
                });
            }
            if path[0] == "set" {
                if let Some(title) = title {
                    return Some(UrlCommand::ShowSet { title });
                }
            }
        }
        let arguments = parameters
            .get("value")
            .and_then(|v| v.parse::<f32>().ok())
            .map(|v| vec![OscArgument::Float(v)])
            .unwrap_or_default();
        let message = OscMessage::new(format!("/{}", path.join("/")), arguments);
        OscCommand::parse(&message).map(UrlCommand::Osc)
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_by_name() {
        assert_eq!(
            UrlCommand::parse("soundboard://play?title=Bonjour%20toi&set=Effets"),
            Some(UrlCommand::Tile {
                title: "Bonjour toi".into(),
                set: Some("Effets".into()),
                action: LinkTileAction::Play,
            })
        );
        assert_eq!(
            UrlCommand::parse("SOUNDBOARD://fade/?title=A"),
            Some(UrlCommand::Tile {
                title: "A".into(),
                set: None,
                action: LinkTileAction::Fade,
            })
        );
        assert_eq!(
            UrlCommand::parse("soundboard://play"),
            None,
            "a title is needed"
        );
    }

    #[test]
    fn double_encoded_names_are_decoded() {
        assert_eq!(
            UrlCommand::parse("soundboard://stop?title=Caf%25C3%25A9"),
            Some(UrlCommand::Tile {
                title: "Café".into(),
                set: None,
                action: LinkTileAction::Stop,
            })
        );
    }

    #[test]
    fn show_set_and_osc_paths() {
        assert_eq!(
            UrlCommand::parse("soundboard://set?title=Effets"),
            Some(UrlCommand::ShowSet {
                title: "Effets".into()
            })
        );
        assert_eq!(
            UrlCommand::parse("soundboard://transport/stopAll"),
            Some(UrlCommand::Osc(OscCommand::StopAll))
        );
        assert_eq!(
            UrlCommand::parse("soundboard://master/volume?value=0.5"),
            Some(UrlCommand::Osc(OscCommand::MasterVolume(0.5)))
        );
        assert!(matches!(
            UrlCommand::parse("soundboard://set/0/tile/1/0/play"),
            Some(UrlCommand::Osc(OscCommand::Tile { .. }))
        ));
        assert_eq!(UrlCommand::parse("https://play?title=A"), None);
        assert_eq!(UrlCommand::parse("soundboard://nowhere"), None);
    }

    #[test]
    fn stray_percent_signs_stay() {
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
    }
}
