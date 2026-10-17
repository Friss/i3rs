//! Track event annotations use section meaning and actual event positions.
use crate::track::SectionKind;
use crate::{Lap, Sector, TrackData};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq)]
pub enum AnnotationKind {
    MinimumSpeed(f64),
    MaximumSpeed(f64),
    GearChange { from: i64, to: i64 },
}
#[derive(Clone, Debug, PartialEq)]
pub struct TrackAnnotation {
    pub time: f64,
    pub x: f64,
    pub y: f64,
    pub section: String,
    pub kind: AnnotationKind,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AnnotationSettings {
    pub section_names: bool,
    pub corner_minima: bool,
    pub straight_maxima: bool,
    pub gear_changes: bool,
    pub speed_unit: String,
    pub precision: u8,
}
impl Default for AnnotationSettings {
    fn default() -> Self {
        Self {
            section_names: true,
            corner_minima: true,
            straight_maxima: true,
            gear_changes: true,
            speed_unit: "mph".into(),
            precision: 1,
        }
    }
}
pub fn point_at_time(track: &TrackData, time: f64) -> Option<(f64, f64)> {
    let i = track.time.partition_point(|t| *t < time);
    if i < track.time.len() && track.time[i] == time {
        return track
            .x
            .get(i)
            .zip(track.y.get(i))
            .filter(|(x, y)| x.is_finite() && y.is_finite())
            .map(|(x, y)| (*x, *y));
    }
    if i == 0 || i >= track.time.len() {
        return None;
    }
    let (a, b) = (i - 1, i);
    let span = track.time[b] - track.time[a];
    if span <= 0.
        || ![track.x[a], track.x[b], track.y[a], track.y[b]]
            .iter()
            .all(|v| v.is_finite())
    {
        return None;
    }
    let f = (time - track.time[a]) / span;
    Some((
        track.x[a] + f * (track.x[b] - track.x[a]),
        track.y[a] + f * (track.y[b] - track.y[a]),
    ))
}
pub fn track_annotations(
    track: &TrackData,
    lap: &Lap,
    sectors: &[Sector],
    gear: Option<(&[f64], f64)>,
    settings: &AnnotationSettings,
) -> Vec<TrackAnnotation> {
    let mut result = vec![];
    if let Some((speed, freq)) = &track.corrected_speed
        && *freq > 0.0
    {
        for sector in sectors {
            let maximum = match sector.semantics.kind {
                SectionKind::Straight if settings.straight_maxima => true,
                SectionKind::Corner if settings.corner_minima => false,
                _ => continue,
            };
            let Some((start, end)) = crate::sector_interval(sector, lap, track) else {
                continue;
            };
            let ranges = if end > start {
                vec![(start, end)]
            } else {
                vec![(start, lap.end_time), (lap.start_time, end)]
            };
            let mut candidates: Vec<(f64, f64)> = vec![];
            for (a, b) in ranges {
                let window = crate::sample_window_for_time(*freq, speed.len(), a, b);
                for (i, &v) in speed
                    .iter()
                    .enumerate()
                    .take(window.end_sample)
                    .skip(window.start_sample)
                {
                    if v.is_finite() {
                        candidates.push((i as f64 / *freq, v));
                    }
                }
                // Continuous extrema include the exact section start, not a sample before it.
                if let Some(v) = crate::sample_at_time(speed, *freq, a) {
                    candidates.push((a, v));
                }
            }
            let best = candidates.into_iter().min_by(|a, b| {
                let order = if maximum {
                    b.1.total_cmp(&a.1)
                } else {
                    a.1.total_cmp(&b.1)
                };
                order.then_with(|| a.0.total_cmp(&b.0))
            });
            if let Some((time, value)) = best
                && let Some((x, y)) = point_at_time(track, time)
            {
                result.push(TrackAnnotation {
                    time,
                    x,
                    y,
                    section: sector.semantics.id.clone(),
                    kind: if maximum {
                        AnnotationKind::MaximumSpeed(value)
                    } else {
                        AnnotationKind::MinimumSpeed(value)
                    },
                });
            }
        }
    }
    if settings.gear_changes
        && let Some((data, freq)) = gear
        && freq > 0.0
    {
        for i in 1..data.len() {
            let (a, b) = (data[i - 1], data[i]);
            let time = i as f64 / freq;
            if time < lap.start_time
                || time >= lap.end_time
                || !a.is_finite()
                || !b.is_finite()
                || a.fract() != 0.
                || b.fract() != 0.
                || a == b
            {
                continue;
            }
            if let Some((x, y)) = point_at_time(track, time) {
                result.push(TrackAnnotation {
                    time,
                    x,
                    y,
                    section: String::new(),
                    kind: AnnotationKind::GearChange {
                        from: a as i64,
                        to: b as i64,
                    },
                });
            }
        }
    }
    result.sort_by(|a, b| a.time.total_cmp(&b.time));
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::track::{DistanceSection, SectionSemantics};
    #[test]
    fn annotations_use_categories_wrapping_boundaries_and_event_positions() {
        let mut track = TrackData::from_normalized_parts(
            (0..9).map(|v| v as f64).collect(),
            vec![0.; 9],
            (0..9).map(|v| v as f64).collect(),
            1.0,
        );
        track.distance = Some(((0..9).map(|v| v as f64 * 10.).collect(), 1.0));
        track.corrected_speed = Some((vec![10., 20., 40., 30., 10., 5., 8., 25., 10.], 1.0));
        let sector = |id: &str, kind, start, end| Sector {
            name: id.into(),
            semantics: SectionSemantics {
                id: id.into(),
                kind,
                categories: Default::default(),
            },
            start_index: 0,
            end_index: 1,
            distance: Some(DistanceSection {
                start_mm: start,
                end_mm: end,
                track_length_mm: 80_000,
            }),
        };
        let lap = Lap {
            number: 1,
            name: "Lap".into(),
            start_time: 0.,
            end_time: 8.,
        };
        let sectors = [
            sector("straight", SectionKind::Straight, 5_000, 35_000),
            sector("corner", SectionKind::Corner, 35_000, 65_000),
            sector("wrap", SectionKind::Straight, 65_000, 5_000),
        ];
        let events = track_annotations(
            &track,
            &lap,
            &sectors,
            Some((&[1., 1., 2., 2., f64::NAN, 3., 3., 4., 4.], 1.0)),
            &Default::default(),
        );
        assert!(
            events
                .iter()
                .any(|e| e.time == 2. && e.x == 2. && e.kind == AnnotationKind::MaximumSpeed(40.))
        );
        assert!(
            events
                .iter()
                .any(|e| e.time == 5. && e.kind == AnnotationKind::MinimumSpeed(5.))
        );
        assert!(events.iter().any(|e| e.time == 7. && e.section == "wrap"));
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e.kind, AnnotationKind::GearChange { .. }))
                .count(),
            2
        );
        track.x[2] = f64::NAN;
        assert!(
            !track_annotations(&track, &lap, &sectors, None, &Default::default())
                .iter()
                .any(|e| e.time == 2.)
        );
    }
}
