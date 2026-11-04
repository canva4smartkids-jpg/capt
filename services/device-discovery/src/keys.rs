//! `TvRemoteKey` (packages/capture-contracts/src/desktop-ipc.ts) and its mapping per protocol.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TvRemoteKey {
    Up,
    Down,
    Left,
    Right,
    Enter,
    Back,
    Home,
    Menu,
    VolumeUp,
    VolumeDown,
    Mute,
    Play,
    Pause,
    Stop,
    Power,
}

pub const ALL_KEYS: [TvRemoteKey; 15] = [
    TvRemoteKey::Up,
    TvRemoteKey::Down,
    TvRemoteKey::Left,
    TvRemoteKey::Right,
    TvRemoteKey::Enter,
    TvRemoteKey::Back,
    TvRemoteKey::Home,
    TvRemoteKey::Menu,
    TvRemoteKey::VolumeUp,
    TvRemoteKey::VolumeDown,
    TvRemoteKey::Mute,
    TvRemoteKey::Play,
    TvRemoteKey::Pause,
    TvRemoteKey::Stop,
    TvRemoteKey::Power,
];

/// How a key is delivered over LG SSAP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LgKeyAction {
    /// `type:button\nname:<NAME>\n\n` on the pointer input socket.
    Button(&'static str),
    /// A plain SSAP request with an empty payload.
    Request(&'static str),
}

pub fn lg_action(key: TvRemoteKey) -> LgKeyAction {
    use LgKeyAction::*;
    match key {
        TvRemoteKey::Up => Button("UP"),
        TvRemoteKey::Down => Button("DOWN"),
        TvRemoteKey::Left => Button("LEFT"),
        TvRemoteKey::Right => Button("RIGHT"),
        TvRemoteKey::Enter => Button("ENTER"),
        TvRemoteKey::Back => Button("BACK"),
        TvRemoteKey::Home => Button("HOME"),
        TvRemoteKey::Menu => Button("MENU"),
        TvRemoteKey::Mute => Button("MUTE"),
        TvRemoteKey::VolumeUp => Request("ssap://audio/volumeUp"),
        TvRemoteKey::VolumeDown => Request("ssap://audio/volumeDown"),
        TvRemoteKey::Play => Request("ssap://media.controls/play"),
        TvRemoteKey::Pause => Request("ssap://media.controls/pause"),
        TvRemoteKey::Stop => Request("ssap://media.controls/stop"),
        // SSAP can only turn the TV off; powering on needs Wake-on-LAN.
        TvRemoteKey::Power => Request("ssap://system/turnOff"),
    }
}

/// Samsung `ms.remote.control` key codes.
pub fn samsung_key(key: TvRemoteKey) -> &'static str {
    match key {
        TvRemoteKey::Up => "KEY_UP",
        TvRemoteKey::Down => "KEY_DOWN",
        TvRemoteKey::Left => "KEY_LEFT",
        TvRemoteKey::Right => "KEY_RIGHT",
        TvRemoteKey::Enter => "KEY_ENTER",
        TvRemoteKey::Back => "KEY_RETURN",
        TvRemoteKey::Home => "KEY_HOME",
        TvRemoteKey::Menu => "KEY_MENU",
        TvRemoteKey::VolumeUp => "KEY_VOLUP",
        TvRemoteKey::VolumeDown => "KEY_VOLDOWN",
        TvRemoteKey::Mute => "KEY_MUTE",
        TvRemoteKey::Play => "KEY_PLAY",
        TvRemoteKey::Pause => "KEY_PAUSE",
        TvRemoteKey::Stop => "KEY_STOP",
        TvRemoteKey::Power => "KEY_POWER",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spellings_match_typescript_union() {
        // Literal union from desktop-ipc.ts `TvRemoteKey`.
        let ts = ["UP", "DOWN", "LEFT", "RIGHT", "ENTER", "BACK", "HOME", "MENU", "VOLUME_UP", "VOLUME_DOWN", "MUTE", "PLAY", "PAUSE", "STOP", "POWER"];
        let src = include_str!("../../../packages/capture-contracts/src/desktop-ipc.ts");
        for (k, name) in ALL_KEYS.iter().zip(ts) {
            assert_eq!(serde_json::to_value(k).unwrap(), name);
            assert!(src.contains(&format!("\"{name}\"")), "{name} missing from desktop-ipc.ts");
        }
    }

    #[test]
    fn every_key_maps() {
        for k in ALL_KEYS {
            assert!(samsung_key(k).starts_with("KEY_"));
            match lg_action(k) {
                LgKeyAction::Button(b) => assert!(!b.is_empty() && b.chars().all(|c| c.is_ascii_uppercase())),
                LgKeyAction::Request(r) => assert!(r.starts_with("ssap://")),
            }
        }
        assert_eq!(samsung_key(TvRemoteKey::Back), "KEY_RETURN");
        assert_eq!(lg_action(TvRemoteKey::Up), LgKeyAction::Button("UP"));
    }
}
