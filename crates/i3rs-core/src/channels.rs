//! Portable channel references, source policies and analysis metadata.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Interpolation {
    #[default]
    Linear,
    Previous,
    Nearest,
}

/// A reference retains its requested name even when its source is unavailable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelReference {
    pub name: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub recording: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChannelKey {
    Physical(usize),
    Derived(u64),
    Constant(usize),
    Generated(String),
}

#[derive(Clone, Debug)]
pub struct ChannelCandidate {
    pub name: String,
    pub source: String,
    pub recording: Option<String>,
    pub key: ChannelKey,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChannelResolution {
    Resolved {
        key: ChannelKey,
        name: String,
        source: String,
    },
    Unresolved {
        reference: ChannelReference,
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourcePolicy {
    pub name: String,
    pub enabled: bool,
    /// Smaller numbers take precedence; equal priority is ordered by source name.
    pub priority: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChannelMapping {
    pub original: String,
    pub name: String,
    /// Metadata override only; value conversion is a separate display transform.
    pub unit: Option<String>,
    #[serde(default = "identity_scale")]
    pub display_scale: f64,
    pub display_offset: f64,
    pub display_unit: Option<String>,
    pub recording: Option<String>,
    pub precision: Option<i16>,
    pub interpolation: Interpolation,
    pub comment: String,
    pub enum_labels: BTreeMap<i64, String>,
}

fn identity_scale() -> f64 {
    1.0
}
impl Default for ChannelMapping {
    fn default() -> Self {
        Self {
            original: String::new(),
            name: String::new(),
            unit: None,
            display_scale: 1.0,
            display_offset: 0.0,
            display_unit: None,
            recording: None,
            precision: None,
            interpolation: Interpolation::Linear,
            comment: String::new(),
            enum_labels: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MathOptions {
    pub source: String,
    pub recording: Option<String>,
    pub quantity: String,
    pub display_unit: Option<String>,
    pub comment: String,
    pub enum_labels: BTreeMap<i64, String>,
    pub output_freq: Option<u16>,
    pub interpolation: Interpolation,
}
impl Default for MathOptions {
    fn default() -> Self {
        Self {
            source: "User".into(),
            recording: None,
            quantity: String::new(),
            display_unit: None,
            comment: String::new(),
            enum_labels: BTreeMap::new(),
            output_freq: None,
            interpolation: Interpolation::Linear,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SetupConstant {
    pub name: String,
    pub value: f64,
    pub unit: String,
    pub recording: String,
    #[serde(default)]
    pub comment: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChannelSettings {
    pub version: u32,
    pub aliases: BTreeMap<String, Vec<String>>,
    pub sources: Vec<SourcePolicy>,
    pub mappings: Vec<ChannelMapping>,
    pub constants: Vec<SetupConstant>,
}
impl Default for ChannelSettings {
    fn default() -> Self {
        Self {
            version: 1,
            aliases: BTreeMap::new(),
            mappings: Vec::new(),
            constants: Vec::new(),
            sources: [
                ("Temporary", 0),
                ("Log File", 10),
                ("User", 20),
                ("MoTeC", 30),
                ("Logged", 40),
            ]
            .into_iter()
            .map(|(name, priority)| SourcePolicy {
                name: name.into(),
                enabled: true,
                priority,
            })
            .collect(),
        }
    }
}

pub fn normalized_name(name: &str) -> String {
    name.to_lowercase()
        .replace(['.', '_'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Deterministic matching: literal name first, then normalized name, then lexical order.
pub fn resolve_name<'a>(
    reference: &str,
    names: impl IntoIterator<Item = &'a str>,
) -> Option<&'a str> {
    let normalized = normalized_name(reference);
    names
        .into_iter()
        .filter(|name| normalized_name(name) == normalized)
        .min_by_key(|name| (*name != reference, *name))
}

impl ChannelSettings {
    pub fn source(&self, name: &str) -> Option<&SourcePolicy> {
        self.sources.iter().find(|s| s.name == name)
    }
    pub fn enabled(&self, source: &str) -> bool {
        self.source(source).is_some_and(|s| s.enabled)
    }
    pub fn mapping(&self, original: &str, recording: &str) -> Option<&ChannelMapping> {
        self.mappings
            .iter()
            .filter(|m| {
                m.original == original && m.recording.as_deref().is_none_or(|r| r == recording)
            })
            .min_by_key(|m| m.recording.is_none())
    }
    pub fn resolve(
        &self,
        reference: &ChannelReference,
        recording: &str,
        catalog: &[ChannelCandidate],
    ) -> ChannelResolution {
        if reference
            .recording
            .as_deref()
            .is_some_and(|r| r != recording)
        {
            return ChannelResolution::Unresolved {
                reference: reference.clone(),
                reason: "reference belongs to another recording".into(),
            };
        }
        let targets = if reference.source.is_some() {
            // A qualified source/name pins the actual channel, including saved selections.
            vec![reference.name.clone()]
        } else {
            resolve_name(&reference.name, self.aliases.keys().map(String::as_str))
                .and_then(|name| self.aliases.get(name))
                .cloned()
                .unwrap_or_else(|| vec![reference.name.clone()])
        };
        for target in &targets {
            let (target_source, target_name) = target
                .split_once("::")
                .map_or((None, target.as_str()), |(s, n)| (Some(s), n));
            let normalized = normalized_name(target_name);
            let selected = catalog
                .iter()
                .filter(|c| {
                    normalized_name(&c.name) == normalized
                        && target_source.is_none_or(|s| s == c.source)
                        && self.enabled(&c.source)
                        && c.recording.as_deref().is_none_or(|r| r == recording)
                        && reference.source.as_deref().is_none_or(|s| s == c.source)
                })
                .min_by_key(|c| {
                    (
                        self.source(&c.source).map_or(i32::MAX, |s| s.priority),
                        c.name != target_name,
                        &c.source,
                        &c.name,
                    )
                });
            if let Some(c) = selected {
                return ChannelResolution::Resolved {
                    key: c.key.clone(),
                    name: c.name.clone(),
                    source: c.source.clone(),
                };
            }
        }
        ChannelResolution::Unresolved {
            reference: reference.clone(),
            reason: format!(
                "no enabled source for '{}' (tried {})",
                reference.name,
                targets.join(", ")
            ),
        }
    }
}

/// Component gates include only finite positive values. Expression truth rules are separate.
pub fn component_gate_includes(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

/// Sample a result with an explicit interpolation policy, retaining missing data.
pub fn sample_with_policy(
    data: &[f64],
    freq: u16,
    time: f64,
    policy: Interpolation,
) -> Option<f64> {
    if freq == 0 || !time.is_finite() || time < 0.0 {
        return None;
    }
    sample_at_position_with_policy(data, time * freq as f64, policy)
}

pub(crate) fn sample_at_position_with_policy(
    data: &[f64],
    position: f64,
    policy: Interpolation,
) -> Option<f64> {
    if !position.is_finite() || position < 0.0 {
        return None;
    }
    match policy {
        Interpolation::Linear => crate::analysis::sample_at_position(data, position),
        Interpolation::Previous | Interpolation::Nearest => {
            let index = if policy == Interpolation::Previous {
                position.floor()
            } else {
                position.round()
            };
            data.get(index as usize).copied().filter(|v| v.is_finite())
        }
    }
}

/// Supported physical unit conversions, via an SI scale and offset. Unknown units
/// may pass through only when identical; incompatible dimensions are errors.
pub fn unit_transform(from: &str, to: &str) -> Result<(f64, f64), String> {
    fn base(unit: &str) -> Option<(&'static str, f64, f64)> {
        // Lowercase g denotes grams, not the acceleration unit G.
        if unit.trim() == "g" {
            return None;
        }
        match unit.to_lowercase().replace(' ', "").as_str() {
            "m/s" => Some(("speed", 1.0, 0.0)),
            "km/h" | "kph" => Some(("speed", 1.0 / 3.6, 0.0)),
            "mph" => Some(("speed", 0.44704, 0.0)),
            "m/s/s" | "m/s²" | "m/s^2" => Some(("acceleration", 1.0, 0.0)),
            "g" => Some(("acceleration", 9.80665, 0.0)),
            "rad" => Some(("angle", 1.0, 0.0)),
            "deg" | "°" => Some(("angle", std::f64::consts::PI / 180.0, 0.0)),
            "m" => Some(("length", 1.0, 0.0)),
            "mm" => Some(("length", 0.001, 0.0)),
            "ft" => Some(("length", 0.3048, 0.0)),
            "pa" => Some(("pressure", 1.0, 0.0)),
            "kpa" => Some(("pressure", 1000.0, 0.0)),
            "bar" => Some(("pressure", 100000.0, 0.0)),
            "psi" => Some(("pressure", 6894.757293168, 0.0)),
            "c" | "°c" | "degc" => Some(("temperature", 1.0, 273.15)),
            "f" | "°f" | "degf" => Some(("temperature", 5.0 / 9.0, 273.15 - 32.0 * 5.0 / 9.0)),
            "k" => Some(("temperature", 1.0, 0.0)),
            _ => None,
        }
    }
    if from == to {
        return Ok((1.0, 0.0));
    }
    match (base(from), base(to)) {
        (Some((a, sa, oa)), Some((b, sb, ob))) if a == b => Ok((sa / sb, (oa - ob) / sb)),
        _ => Err(format!("cannot convert unit '{from}' to '{to}'")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn candidate(name: &str, source: &str, key: ChannelKey) -> ChannelCandidate {
        ChannelCandidate {
            name: name.into(),
            source: source.into(),
            recording: None,
            key,
        }
    }
    fn reference(name: &str) -> ChannelReference {
        ChannelReference {
            name: name.into(),
            source: None,
            recording: None,
        }
    }
    #[test]
    fn aliases_fall_back_in_order_with_enabled_sources_and_recording_scope() {
        let mut settings = ChannelSettings::default();
        settings.aliases.insert(
            "Speed".into(),
            vec![
                "Missing Speed".into(),
                "Wheel Speed".into(),
                "GPS Speed".into(),
            ],
        );
        let catalog = vec![
            candidate("Wheel Speed", "User", ChannelKey::Derived(42)),
            candidate("GPS Speed", "Logged", ChannelKey::Physical(3)),
        ];
        assert!(matches!(
            settings.resolve(&reference("Speed"), "A", &catalog),
            ChannelResolution::Resolved {
                key: ChannelKey::Derived(42),
                ..
            }
        ));
        settings
            .sources
            .iter_mut()
            .find(|s| s.name == "User")
            .unwrap()
            .enabled = false;
        assert!(matches!(
            settings.resolve(&reference("speed"), "A", &catalog),
            ChannelResolution::Resolved {
                key: ChannelKey::Physical(3),
                ..
            }
        ));
        let mut scoped = catalog[1].clone();
        scoped.recording = Some("B".into());
        assert!(matches!(
            settings.resolve(&reference("Speed"), "A", &[scoped]),
            ChannelResolution::Unresolved { .. }
        ));
    }
    #[test]
    fn source_precedence_and_ties_are_deterministic() {
        let settings = ChannelSettings::default();
        let mut catalog = vec![
            candidate("RPM", "Logged", ChannelKey::Physical(0)),
            candidate("RPM", "User", ChannelKey::Derived(2)),
            candidate("RPM", "Temporary", ChannelKey::Derived(9)),
        ];
        let expected = settings.resolve(&reference("RPM"), "A", &catalog);
        assert!(matches!(
            expected,
            ChannelResolution::Resolved {
                key: ChannelKey::Derived(9),
                ..
            }
        ));
        catalog.reverse();
        assert_eq!(settings.resolve(&reference("RPM"), "A", &catalog), expected);
        let mut logged = reference("RPM");
        logged.source = Some("Logged".into());
        assert!(matches!(
            settings.resolve(&logged, "A", &catalog),
            ChannelResolution::Resolved {
                key: ChannelKey::Physical(0),
                ..
            }
        ));
        assert_eq!(resolve_name("a b", ["A_B", "a.b", "a b"]), Some("a b"));
    }
    #[test]
    fn source_qualified_alias_targets_do_not_bind_to_higher_priority_sources() {
        let mut settings = ChannelSettings::default();
        settings
            .aliases
            .insert("Speed".into(), vec!["Logged::GPS Speed".into()]);
        let catalog = vec![
            candidate("GPS Speed", "User", ChannelKey::Derived(8)),
            candidate("GPS Speed", "Logged", ChannelKey::Physical(3)),
        ];
        assert!(matches!(
            settings.resolve(&reference("Speed"), "A", &catalog),
            ChannelResolution::Resolved {
                key: ChannelKey::Physical(3),
                ..
            }
        ));
    }

    #[test]
    fn explicit_source_references_are_not_shadowed_by_aliases() {
        let mut settings = ChannelSettings::default();
        settings.aliases.insert("RPM".into(), vec!["Other".into()]);
        let catalog = vec![
            candidate("RPM", "User", ChannelKey::Derived(8)),
            candidate("Other", "Logged", ChannelKey::Physical(3)),
        ];
        let mut selected = reference("RPM");
        selected.source = Some("User".into());
        assert!(matches!(
            settings.resolve(&selected, "A", &catalog),
            ChannelResolution::Resolved {
                key: ChannelKey::Derived(8),
                ..
            }
        ));
    }

    #[test]
    fn gate_and_unit_conversion_contracts_are_explicit() {
        assert_eq!(
            [
                f64::NEG_INFINITY,
                -2.0,
                -0.0,
                0.0,
                2.0,
                f64::INFINITY,
                f64::NAN
            ]
            .map(component_gate_includes),
            [false, false, false, false, true, false, false]
        );
        for (from, to, raw, expected) in [
            ("km/h", "m/s", 72.0, 20.0),
            ("G", "m/s/s", 1.0, 9.80665),
            ("deg", "rad", 180.0, std::f64::consts::PI),
            ("C", "°F", 100.0, 212.0),
        ] {
            let (scale, offset) = unit_transform(from, to).unwrap();
            assert!((raw * scale + offset - expected).abs() < 1e-10);
        }
        assert!(unit_transform("G", "rad").is_err());
        assert!(unit_transform("g", "m/s/s").is_err());
        assert!(unit_transform("", "m").is_err());
    }
    #[test]
    fn settings_and_reference_metadata_round_trip_with_legacy_defaults() {
        let old: MathOptions = serde_json::from_str("{}").unwrap();
        assert_eq!(old, MathOptions::default());
        let mapping: ChannelMapping = serde_json::from_str("{\"original\":\"Speed\"}").unwrap();
        assert_eq!(mapping.display_scale, 1.0);
        let mut settings = ChannelSettings::default();
        settings.constants.push(SetupConstant {
            name: "Wheelbase".into(),
            value: 2.7,
            unit: "m".into(),
            recording: "A".into(),
            comment: "Measured".into(),
        });
        settings.mappings.push(ChannelMapping {
            original: "Gear".into(),
            enum_labels: BTreeMap::from([(-1, "Reverse".into()), (0, "Neutral".into())]),
            ..Default::default()
        });
        let json = serde_json::to_string(&settings).unwrap();
        assert_eq!(
            serde_json::from_str::<ChannelSettings>(&json).unwrap(),
            settings
        );
    }
}
