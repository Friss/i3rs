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
    pub options: AppOptions,
    #[serde(default)]
    pub channel_preferences: HashMap<String, ChannelPreference>,
}

impl Default for AppPreferences {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::System,
            options: Default::default(),
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

        write_preferences(&path, preferences)
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    #[default]
    English,
    French,
    Japanese,
}
impl Language {
    pub fn text(self, en: &'static str, fr: &'static str, ja: &'static str) -> &'static str {
        match self {
            Self::English => en,
            Self::French => fr,
            Self::Japanese => ja,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AppOptions {
    pub language: Language,
    pub font_size: f32,
    pub monospace: bool,
    pub text_color: Option<[u8; 3]>,
    pub background_color: Option<[u8; 3]>,
    pub shortcuts: Vec<ShortcutBinding>,
}
impl Default for AppOptions {
    fn default() -> Self {
        Self {
            language: Default::default(),
            font_size: 14.,
            monospace: false,
            text_color: None,
            background_color: None,
            shortcuts: default_shortcuts(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShortcutBinding {
    pub action: String,
    pub key: String,
    pub command: bool,
    pub shift: bool,
    pub alt: bool,
}
impl ShortcutBinding {
    pub fn shortcut(&self) -> Option<eframe::egui::KeyboardShortcut> {
        use eframe::egui::{Key, KeyboardShortcut, Modifiers};
        Some(KeyboardShortcut::new(
            Modifiers {
                command: self.command,
                shift: self.shift,
                alt: self.alt,
                ..Default::default()
            },
            Key::from_name(&self.key)?,
        ))
    }
}
fn default_shortcuts() -> Vec<ShortcutBinding> {
    [
        ("Open", "O", true, false),
        ("Save project", "S", true, false),
        ("Load project", "O", true, true),
        ("Save workspace", "S", true, true),
        ("Channels", "B", true, false),
        ("Readout", "R", true, false),
        ("Maths", "M", true, false),
        ("Details", "I", true, false),
        ("Options", "Comma", true, false),
        ("Graph", "G", true, false),
        ("Track map", "T", true, true),
        ("Histogram", "H", true, true),
        ("Scatter", "X", true, true),
        ("FFT", "F", true, true),
        ("Previous worksheet", "F11", false, false),
        ("Next worksheet", "F12", false, false),
        ("Help", "F1", false, false),
    ]
    .into_iter()
    .map(|(action, key, command, shift)| ShortcutBinding {
        action: action.into(),
        key: key.into(),
        command,
        shift,
        alt: false,
    })
    .collect()
}
impl AppOptions {
    pub fn apply(&self, ctx: &eframe::egui::Context) {
        use eframe::egui::*;
        ctx.all_styles_mut(|style| {
            let family = if self.monospace {
                FontFamily::Monospace
            } else {
                FontFamily::Proportional
            };
            for (kind, font) in &mut style.text_styles {
                let size = match kind {
                    TextStyle::Heading => self.font_size * 1.4,
                    TextStyle::Small => self.font_size * 0.8,
                    _ => self.font_size,
                };
                *font = FontId::new(size.clamp(8., 48.), family.clone());
            }
            if let Some(c) = self.text_color {
                style.visuals.override_text_color = Some(Color32::from_rgb(c[0], c[1], c[2]));
            }
            if let Some(c) = self.background_color {
                style.visuals.panel_fill = Color32::from_rgb(c[0], c[1], c[2]);
                style.visuals.window_fill = style.visuals.panel_fill;
            }
        });
    }
    pub fn controls(&mut self, ui: &mut eframe::egui::Ui) {
        use eframe::egui::*;
        let l = self.language;
        ui.horizontal(|ui| {
            ui.label(l.text("Language", "Langue", "言語"));
            for (value, name) in [
                (Language::English, "English"),
                (Language::French, "Français"),
                (Language::Japanese, "日本語"),
            ] {
                ui.selectable_value(&mut self.language, value, name);
            }
        });
        ui.horizontal(|ui| {
            ui.label(l.text("Font size", "Taille de police", "文字サイズ"));
            ui.add(DragValue::new(&mut self.font_size).range(8.0..=32.0));
            ui.checkbox(
                &mut self.monospace,
                l.text("Monospace", "Chasse fixe", "等幅"),
            );
        });
        for (name, color) in [
            (
                l.text("Text colour", "Couleur du texte", "文字色"),
                &mut self.text_color,
            ),
            (
                l.text("Background colour", "Couleur du fond", "背景色"),
                &mut self.background_color,
            ),
        ] {
            ui.horizontal(|ui| {
                let mut enabled = color.is_some();
                if ui.checkbox(&mut enabled, name).changed() {
                    *color = enabled.then_some([128, 128, 128]);
                }
                if let Some(c) = color {
                    ui.color_edit_button_srgb(c);
                }
            });
        }
        ui.collapsing(
            l.text("Keyboard shortcuts", "Raccourcis clavier", "キーボード操作"),
            |ui| {
                ui.label(l.text(
                    "Key name (O, Comma, F11…); Command uses Ctrl on Windows/Linux.",
                    "Nom de touche (O, Comma, F11…) ; Command utilise Ctrl sous Windows/Linux.",
                    "キー名 (O, Comma, F11…)。Windows/Linux では Command は Ctrl です。",
                ));
                Grid::new("shortcut-editor").show(ui, |ui| {
                    for binding in &mut self.shortcuts {
                        ui.label(&binding.action);
                        ui.add(TextEdit::singleline(&mut binding.key).desired_width(80.));
                        ui.checkbox(&mut binding.command, "Command");
                        ui.checkbox(&mut binding.shift, "Shift");
                        ui.checkbox(&mut binding.alt, "Alt");
                        if binding.shortcut().is_none() {
                            ui.colored_label(Color32::YELLOW, "Invalid key");
                        }
                        ui.end_row();
                    }
                });
                if ui
                    .button(l.text(
                        "Reset shortcuts",
                        "Rétablir les raccourcis",
                        "初期設定に戻す",
                    ))
                    .clicked()
                {
                    self.shortcuts = default_shortcuts();
                }
            },
        );
    }
}

/// Small licensed fallback covers the Japanese UI without an OS font dependency.
pub fn install_fonts(ctx: &eframe::egui::Context) {
    use eframe::egui::*;
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "i3rs-ui-jp".into(),
        std::sync::Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/i3rs-ui-jp.ttf"
        ))),
    );
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push("i3rs-ui-jp".into());
    }
    ctx.set_fonts(fonts);
}

/// One writer preserves edit order and coalesces rapid font/colour adjustments.
#[cfg(not(target_arch = "wasm32"))]
pub fn queue_save(
    preferences: AppPreferences,
    messages: crate::channel_analysis::AnalysisMessages,
    session: u64,
) {
    type Request = (
        AppPreferences,
        crate::channel_analysis::AnalysisMessages,
        u64,
    );
    static WRITER: std::sync::OnceLock<std::sync::mpsc::Sender<Request>> =
        std::sync::OnceLock::new();
    let tx = WRITER.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<Request>();
        std::thread::spawn(move || {
            while let Ok(mut request) = rx.recv() {
                for newer in rx.try_iter() {
                    request = newer;
                }
                if let Err(error) = save_preferences(&request.0) {
                    request
                        .1
                        .record(request.2, format!("Failed to save preferences: {error}"));
                }
            }
        });
        tx
    });
    if let Err(error) = tx.send((preferences, messages.clone(), session)) {
        messages.record(session, format!("Preference writer unavailable: {error}"));
    }
}
#[cfg(not(target_arch = "wasm32"))]
fn write_preferences(path: &std::path::Path, preferences: &AppPreferences) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create preferences directory: {e}"))?;
    }
    let json = serde_json::to_string_pretty(preferences)
        .map_err(|e| format!("Failed to serialize preferences: {e}"))?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&temporary, json).map_err(|e| format!("Failed to write preferences: {e}"))?;
    std::fs::rename(&temporary, path).map_err(|e| format!("Failed to replace preferences: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_preferences_migrate_and_all_shortcuts_are_valid() {
        let old: AppPreferences =
            serde_json::from_str(r#"{"theme":"Dark","channel_preferences":{}}"#).unwrap();
        assert!(old.options.shortcuts.iter().all(|b| b.shortcut().is_some()));
        assert!(old.options.shortcuts.iter().any(|b| b.key == "F11"));
        let mut options = old.options;
        options.language = Language::Japanese;
        options.font_size = 19.;
        options.text_color = Some([1, 2, 3]);
        options.shortcuts[0].key = "F8".into();
        let restored: AppOptions =
            serde_json::from_str(&serde_json::to_string(&options).unwrap()).unwrap();
        assert_eq!(restored.language, Language::Japanese);
        assert_eq!(
            restored.shortcuts[0].shortcut().unwrap().logical_key,
            eframe::egui::Key::F8
        );
        assert_eq!(restored.text_color, Some([1, 2, 3]));
    }
}
