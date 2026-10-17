//! Recording-scoped analysis intervals and markers. Raw log data is never rewritten.
use crate::corrections::CorrectionSettings;
use crate::{Lap, TrackDefinition};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RangeKind {
    #[default]
    Lap,
    OutLap,
    InLap,
    Run,
    Stage,
    Transport,
}
impl RangeKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Lap => "Lap",
            Self::OutLap => "Out lap",
            Self::InLap => "In lap",
            Self::Run => "Run",
            Self::Stage => "Stage",
            Self::Transport => "Transport",
        }
    }
    pub fn competition(self) -> bool {
        matches!(self, Self::Lap | Self::Run | Self::Stage)
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Trust {
    Trusted,
    Untrusted,
    #[default]
    Unknown,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarkerKind {
    Beacon,
    Gps,
    StageStart,
    StageEnd,
    MissedBeacon,
    #[default]
    User,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnalysisRange {
    pub id: String,
    pub name: String,
    pub start: f64,
    pub end: f64,
    #[serde(default)]
    pub kind: RangeKind,
    #[serde(default)]
    pub trust: Trust,
    #[serde(default)]
    pub comment: String,
    /// Optional reaction time for a drag run, in seconds; excluded from elapsed times.
    #[serde(default)]
    pub reaction_time: Option<f64>,
}
impl AnalysisRange {
    pub fn as_lap(&self, number: u32) -> Lap {
        Lap {
            number,
            name: self.name.clone(),
            start_time: self.start,
            end_time: self.end,
        }
    }
    pub fn includes(&self, time: f64) -> bool {
        time >= self.start && time < self.end
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RangeMarker {
    pub id: String,
    pub time: f64,
    pub name: String,
    #[serde(default)]
    pub kind: MarkerKind,
    #[serde(default)]
    pub trust: Trust,
    #[serde(default)]
    pub distance_m: Option<f64>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LogDetails {
    pub driver: String,
    pub venue: String,
    pub vehicle: String,
    pub event: String,
    pub notes: String,
    pub custom: BTreeMap<String, String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GpsGeneration {
    pub gate: [[f64; 2]; 2],
    pub minimum_interval: f64,
    pub kind: RangeKind,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct RecordingAnalysis {
    pub drag_setup: crate::drag::DragSetup,
    pub drag_runs: BTreeMap<String, crate::drag::RunDetails>,
    pub version: u32,
    pub recording: String,
    pub duration: f64,
    pub ranges: Vec<AnalysisRange>,
    pub markers: Vec<RangeMarker>,
    pub details: LogDetails,
    pub corrections: CorrectionSettings,
    pub track: Option<TrackDefinition>,
    pub sectors: Vec<crate::Sector>,
    pub selected_range: Option<String>,
    pub gps_generation: Option<GpsGeneration>,
}
impl Default for RecordingAnalysis {
    fn default() -> Self {
        Self {
            drag_setup: Default::default(),
            drag_runs: Default::default(),
            version: 1,
            recording: String::new(),
            duration: 0.,
            ranges: vec![],
            markers: vec![],
            details: LogDetails::default(),
            corrections: CorrectionSettings::default(),
            track: None,
            sectors: vec![],
            selected_range: None,
            gps_generation: None,
        }
    }
}
impl RecordingAnalysis {
    /// Restore analyses saved before legacy ADL rates used the physical clock.
    /// Identity and the old duration must both match; unrelated or invalid
    /// analyses are rejected without changing this value. Current-clock data
    /// is left alone, so saving and reopening cannot scale timestamps twice.
    pub fn migrate_sample_clock(&mut self, ld: &crate::LdFile) -> Result<bool, String> {
        self.validate()?;
        if self.recording != ld.recording_id() {
            return Err("Analysis sidecar belongs to another recording".into());
        }
        let duration = ld.duration_secs();
        if (self.duration - duration).abs() <= 1e-6 {
            return Ok(false);
        }
        let scale = ld.sample_clock_scale();
        if scale == 1.0 || (self.duration - duration * scale).abs() > 1e-6 {
            return Err("Analysis duration differs from this recording".into());
        }
        let mut migrated = self.clone();
        let time = |t: f64| {
            if t == self.duration {
                duration
            } else {
                t / scale
            }
        };
        migrated.duration = duration;
        for range in &mut migrated.ranges {
            range.start = time(range.start);
            range.end = time(range.end);
        }
        for marker in &mut migrated.markers {
            marker.time = time(marker.time);
        }
        // These offsets were chosen against the old sample timeline. Spatial
        // settings and official reaction/time-slip elapsed seconds are unchanged.
        if let Some(wheel_lock) = &mut migrated.corrections.wheel_lock {
            wheel_lock.lag_seconds /= scale;
        }
        if let crate::corrections::GpsShift::Manual(shift) = &mut migrated.corrections.gps_shift {
            *shift /= scale;
        }
        if let Some(generation) = &mut migrated.gps_generation {
            generation.minimum_interval /= scale;
        }
        migrated.validate()?;
        *self = migrated;
        Ok(true)
    }

    pub fn from_laps(recording: String, duration: f64, laps: &[Lap]) -> Self {
        let ranges = laps
            .iter()
            .enumerate()
            .map(|(i, l)| AnalysisRange {
                id: format!("detected-{i}-{:016x}", l.start_time.to_bits()),
                name: l.name.clone(),
                start: l.start_time,
                end: l.end_time,
                kind: if l.name == "Out Lap" {
                    RangeKind::OutLap
                } else if l.name == "In Lap" {
                    RangeKind::InLap
                } else {
                    RangeKind::Lap
                },
                trust: Trust::Unknown,
                comment: String::new(),
                reaction_time: None,
            })
            .filter(|r| r.end > r.start)
            .collect();
        Self {
            recording,
            duration,
            ranges,
            ..Default::default()
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        self.drag_setup.validate()?;
        for (id, slip) in &self.drag_runs {
            let run = self
                .ranges
                .iter()
                .find(|r| &r.id == id && r.kind == RangeKind::Run)
                .ok_or("Time-slip details must belong to a Run range")?;
            slip.validate(run.end - run.start)?;
        }
        if self.version != 1 {
            return Err("Unsupported recording analysis version".into());
        }
        if self.recording.is_empty() || !self.duration.is_finite() || self.duration < 0. {
            return Err("Invalid recording identity or duration".into());
        }
        let mut ids = BTreeSet::new();
        let mut sorted: Vec<_> = self.ranges.iter().collect();
        sorted.sort_by(|a, b| a.start.total_cmp(&b.start));
        for r in &sorted {
            if r.id.is_empty()
                || !ids.insert(&r.id)
                || r.name.trim().is_empty()
                || ![r.start, r.end].iter().all(|v| v.is_finite())
                || r.start < 0.
                || r.end <= r.start
                || r.end > self.duration
                || r.reaction_time
                    .is_some_and(|v| !v.is_finite() || v < 0. || v > r.end - r.start)
            {
                return Err(
                    "Ranges need unique identities, names and finite recording bounds".into(),
                );
            }
        }
        if sorted.windows(2).any(|r| r[0].end > r[1].start) {
            return Err("Analysis ranges overlap".into());
        }
        let mut ids = BTreeSet::new();
        for m in &self.markers {
            if m.id.is_empty()
                || !ids.insert(&m.id)
                || !m.time.is_finite()
                || m.time < 0.
                || m.time > self.duration
                || m.distance_m.is_some_and(|d| !d.is_finite() || d < 0.)
            {
                return Err("Invalid marker identity, time or distance".into());
            }
        }
        let mut section_ids = BTreeSet::new();
        for sector in &self.sectors {
            if sector.start_index == sector.end_index
                || sector.name.trim().is_empty()
                || sector.distance.as_ref().is_some_and(|d| !d.is_valid())
            {
                return Err("Invalid section name or boundaries".into());
            }
            if !sector.semantics.id.is_empty() && !section_ids.insert(&sector.semantics.id) {
                return Err("Duplicate section identity".into());
            }
        }
        if let Some(track) = &self.track {
            track.validate()?;
        }
        if let Some(g) = &self.gps_generation {
            gps_crossing_markers(&[], &[], g.gate, g.minimum_interval)?;
            if !g.kind.competition() {
                return Err("GPS generation needs a competition range type".into());
            }
        }
        self.corrections.validate()
    }
    pub fn laps(&self) -> Vec<Lap> {
        self.ranges
            .iter()
            .enumerate()
            .map(|(i, r)| r.as_lap(i as u32))
            .collect()
    }
    pub fn mark_missed_beacons(&mut self) {
        for r in &mut self.ranges {
            if self
                .markers
                .iter()
                .any(|m| m.kind == MarkerKind::MissedBeacon && r.includes(m.time))
            {
                r.trust = Trust::Untrusted;
            }
        }
    }
}

/// Boundaries keep exact off-grid times. Incomplete first/last intervals remain distinct.
pub fn ranges_from_markers(
    markers: &[RangeMarker],
    duration: f64,
    kind: RangeKind,
) -> Result<Vec<AnalysisRange>, String> {
    if !duration.is_finite() || duration <= 0. || !kind.competition() {
        return Err("Choose a competition range type and positive duration".into());
    }
    let mut boundaries: Vec<_> = markers
        .iter()
        .filter(|m| m.kind != MarkerKind::MissedBeacon)
        .collect();
    if boundaries
        .iter()
        .any(|m| !m.time.is_finite() || m.time < 0. || m.time > duration)
    {
        return Err("Marker lies outside the recording".into());
    }
    boundaries.sort_by(|a, b| a.time.total_cmp(&b.time));
    if boundaries.windows(2).any(|m| m[0].time == m[1].time) {
        return Err("Duplicate boundary times".into());
    }
    if boundaries.is_empty() {
        return Err("No boundary markers available".into());
    }
    let mut ranges = vec![];
    let mut push = |id: String, name: String, start: f64, end: f64, kind, trust| {
        if end > start {
            ranges.push(AnalysisRange {
                id,
                name,
                start,
                end,
                kind,
                trust,
                comment: String::new(),
                reaction_time: None,
            });
        }
    };
    let first = boundaries[0];
    push(
        format!("before-{}", first.id),
        if kind == RangeKind::Lap {
            "Out Lap".into()
        } else {
            "Transport 1".into()
        },
        0.,
        first.time,
        if kind == RangeKind::Lap {
            RangeKind::OutLap
        } else {
            RangeKind::Transport
        },
        Trust::Unknown,
    );
    let mut competition_number = 0;
    let mut transport_number = usize::from(first.time > 0.);
    for b in boundaries.windows(2) {
        let trust = if b.iter().any(|m| m.trust == Trust::Untrusted) {
            Trust::Untrusted
        } else if b.iter().all(|m| m.trust == Trust::Trusted) {
            Trust::Trusted
        } else {
            Trust::Unknown
        };
        let interval_kind = if kind == RangeKind::Stage
            && boundaries
                .iter()
                .any(|m| matches!(m.kind, MarkerKind::StageStart | MarkerKind::StageEnd))
            && !(b[0].kind == MarkerKind::StageStart && b[1].kind == MarkerKind::StageEnd)
        {
            RangeKind::Transport
        } else {
            kind
        };
        let number = if interval_kind == RangeKind::Transport {
            transport_number += 1;
            transport_number
        } else {
            competition_number += 1;
            competition_number
        };
        push(
            format!("between-{}-{}", b[0].id, b[1].id),
            format!("{} {}", interval_kind.label(), number),
            b[0].time,
            b[1].time,
            interval_kind,
            trust,
        );
    }
    let last = *boundaries.last().unwrap();
    push(
        format!("after-{}", last.id),
        if kind == RangeKind::Lap {
            "In Lap".into()
        } else {
            format!("Transport {}", transport_number + 1)
        },
        last.time,
        duration,
        if kind == RangeKind::Lap {
            RangeKind::InLap
        } else {
            RangeKind::Transport
        },
        Trust::Unknown,
    );
    for r in &mut ranges {
        if markers
            .iter()
            .any(|m| m.kind == MarkerKind::MissedBeacon && r.includes(m.time))
        {
            r.trust = Trust::Untrusted;
        }
    }
    Ok(ranges)
}

/// Directed finite GPS line crossings, interpolated between fixes. Never bridge outages.
pub fn gps_crossing_markers(
    fixes: &[[f64; 2]],
    times: &[f64],
    gate: [[f64; 2]; 2],
    minimum_interval: f64,
) -> Result<Vec<RangeMarker>, String> {
    if fixes.len() != times.len()
        || !minimum_interval.is_finite()
        || minimum_interval < 0.
        || !gate.iter().flatten().all(|v| v.is_finite())
        || gate.iter().any(|p| p[0].abs() > 90. || p[1].abs() > 180.)
    {
        return Err("Invalid GPS gate or sample times".into());
    }
    let cos = gate[0][0].to_radians().cos();
    let vector = |p: [f64; 2]| [(p[1] - gate[0][1]) * cos, p[0] - gate[0][0]];
    let g = vector(gate[1]);
    let length = g[0] * g[0] + g[1] * g[1];
    if length < 1e-14 {
        return Err("GPS gate endpoints must differ".into());
    }
    let side = |p: [f64; 2]| g[0] * p[1] - g[1] * p[0];
    let mut result: Vec<RangeMarker> = vec![];
    for i in 1..fixes.len() {
        let valid = |p: [f64; 2]| {
            p.iter().all(|v| v.is_finite())
                && p[0].abs() <= 90.
                && p[1].abs() <= 180.
                && p != [0., 0.]
        };
        if !valid(fixes[i - 1])
            || !valid(fixes[i])
            || !times[i - 1].is_finite()
            || !times[i].is_finite()
            || times[i] <= times[i - 1]
        {
            continue;
        }
        let a = vector(fixes[i - 1]);
        let b = vector(fixes[i]);
        let (sa, sb) = (side(a), side(b));
        if sa >= 0. || sb < 0. {
            continue;
        }
        let f = sa / (sa - sb);
        let p = [a[0] + f * (b[0] - a[0]), a[1] + f * (b[1] - a[1])];
        let along = (p[0] * g[0] + p[1] * g[1]) / length;
        let t = times[i - 1] + f * (times[i] - times[i - 1]);
        if !(0.0..=1.0).contains(&along)
            || result.last().is_some_and(|m| t - m.time < minimum_interval)
        {
            continue;
        }
        result.push(RangeMarker {
            id: format!("gps-{:016x}", t.to_bits()),
            time: t,
            name: "GPS crossing".into(),
            kind: MarkerKind::Gps,
            trust: Trust::Unknown,
            distance_m: None,
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn marker(t: f64, kind: MarkerKind) -> RangeMarker {
        RangeMarker {
            id: format!("m{t}"),
            time: t,
            name: String::new(),
            kind,
            trust: Trust::Trusted,
            distance_m: None,
        }
    }
    #[test]
    fn off_grid_boundaries_missing_beacons_and_transports_remain_typed() {
        let m = vec![
            marker(1.25, MarkerKind::Beacon),
            marker(5.75, MarkerKind::MissedBeacon),
            marker(9.5, MarkerKind::Beacon),
        ];
        let r = ranges_from_markers(&m, 12., RangeKind::Lap).unwrap();
        assert_eq!(
            (r[0].kind, r[1].kind, r[2].kind),
            (RangeKind::OutLap, RangeKind::Lap, RangeKind::InLap)
        );
        assert_eq!(
            (r[1].start, r[1].end, r[1].trust),
            (1.25, 9.5, Trust::Untrusted)
        );
        let r = ranges_from_markers(&m, 12., RangeKind::Stage).unwrap();
        assert_eq!(
            (r[0].kind, r[1].kind, r[2].kind),
            (RangeKind::Transport, RangeKind::Stage, RangeKind::Transport)
        );
        assert!(!r[0].kind.competition());
        assert!(r[1].kind.competition());
    }
    #[test]
    fn gps_crossings_are_directed_off_grid_and_do_not_bridge_missing_fixes() {
        let gate = [[36., -79.001], [36., -78.999]];
        let fixes = [
            [35.999, -79.],
            [36.001, -79.],
            [f64::NAN, -79.],
            [35.999, -79.],
            [36.001, -79.],
        ];
        let m = gps_crossing_markers(&fixes, &[0., 1., 2., 3., 4.], gate, 2.).unwrap();
        assert_eq!(m.iter().map(|m| m.time).collect::<Vec<_>>(), [0.5, 3.5]);
        assert!(
            gps_crossing_markers(&fixes, &[0., 1., 2., 3., 4.], gate, 4.)
                .unwrap()
                .len()
                == 1
        );
    }
}

/// Analysis selection retains range identity/type/trust while the view zoom changes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RangeSelection {
    pub recording: String,
    pub range_id: Option<String>,
    pub kind: Option<RangeKind>,
    pub trust: Trust,
    pub start: f64,
    pub end: f64,
}

#[cfg(test)]
mod stage_tests {
    use super::*;
    #[test]
    fn stage_start_end_markers_preserve_the_transport_between_stages() {
        let markers = [
            (1., MarkerKind::StageStart),
            (3., MarkerKind::StageEnd),
            (5., MarkerKind::StageStart),
            (7., MarkerKind::StageEnd),
        ]
        .into_iter()
        .map(|(time, kind)| RangeMarker {
            id: format!("{time}"),
            time,
            name: String::new(),
            kind,
            trust: Trust::Trusted,
            distance_m: None,
        })
        .collect::<Vec<_>>();
        let ranges = ranges_from_markers(&markers, 9., RangeKind::Stage).unwrap();
        assert_eq!(ranges[3].name, "Stage 2");
        assert_eq!(ranges[4].name, "Transport 3");
        assert_eq!(
            ranges.iter().map(|r| r.kind).collect::<Vec<_>>(),
            [
                RangeKind::Transport,
                RangeKind::Stage,
                RangeKind::Transport,
                RangeKind::Stage,
                RangeKind::Transport
            ]
        );
    }
}
