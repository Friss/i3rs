//! Application-level preferences persisted outside workspace/project files.

use std::collections::HashMap;
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::state::ChannelPreference;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
    HighContrast,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AppPreferences {
    #[serde(default)]
    pub theme: ThemeChoice,
    #[serde(default)]
    pub channel_preferences: HashMap<String, ChannelPreference>,
}

impl Default for AppPreferences {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::System,
            channel_preferences: HashMap::new(),
        }
    }
}

pub fn load_preferences() -> AppPreferences {
    #[cfg(target_arch = "wasm32")]
    {
        let Some(storage) = local_storage() else {
            return AppPreferences::default();
        };
        let Ok(Some(json)) = storage.get_item("i3rs.preferences") else {
            return AppPreferences::default();
        };
        return serde_json::from_str(&json).unwrap_or_default();
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
    let Some(path) = preferences_path() else {
        return AppPreferences::default();
    };

    let Ok(json) = std::fs::read_to_string(path) else {
        return AppPreferences::default();
    };

        serde_json::from_str(&json).unwrap_or_default()
    }
}

pub fn save_preferences(preferences: &AppPreferences) -> Result<(), String> {
    #[cfg(target_arch = "wasm32")]
    {
        let Some(storage) = local_storage() else {
            return Err("Could not access browser local storage".into());
        };
        let json = serde_json::to_string(preferences)
            .map_err(|e| format!("Failed to serialize preferences: {}", e))?;
        storage
            .set_item("i3rs.preferences", &json)
            .map_err(|e| format!("Failed to write preferences: {:?}", e))
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
    let Some(path) = preferences_path() else {
        return Err("Could not determine preferences path".into());
    };

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create preferences directory: {}", e))?;
    }

    let json = serde_json::to_string_pretty(preferences)
        .map_err(|e| format!("Failed to serialize preferences: {}", e))?;
        std::fs::write(path, json).map_err(|e| format!("Failed to write preferences: {}", e))
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn preferences_path() -> Option<PathBuf> {
    let base = if cfg!(target_os = "macos") {
        home_dir()?.join("Library/Application Support")
    } else if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from)?
    } else if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        PathBuf::from(xdg)
    } else {
        home_dir()?.join(".config")
    };

    Some(base.join("i3rs").join("preferences.json"))
}

#[cfg(not(target_arch = "wasm32"))]
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(target_arch = "wasm32")]
fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}
