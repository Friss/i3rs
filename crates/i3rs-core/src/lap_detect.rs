//! Lap boundary detection from channel data.
//!
//! Detects lap boundaries using "Lap Time Running" resets (preferred, sub-sample
//! precision) or "Lap Number" transitions (fallback). Labels laps as Out Lap,
//! Lap 1/2/…, and In Lap to match MoTeC i2 behaviour.

use crate::ld_parser::{ChannelMeta, LdFile};

/// A detected lap with timing information.
#[derive(Debug, Clone)]
pub struct Lap {
    /// Sequential number: 0 = Out Lap, 1.. = timed laps, last = In Lap.
    pub number: u32,
    /// Display label: "Out Lap", "Lap 1", "In Lap", etc.
    pub name: String,
    pub start_time: f64,
    pub end_time: f64,
}

impl Lap {
    pub fn duration(&self) -> f64 {
        self.end_time - self.start_time
    }
}

/// Format a duration in seconds as `M:SS.mmm` (e.g. `2:08.392`) or `S.mmm`
/// for durations under a minute, matching MoTeC i2's display style.
pub fn format_duration(secs: f64) -> String {
    let mins = (secs / 60.0) as u32;
    let remainder = secs - (mins as f64 * 60.0);
    if mins > 0 {
        format!("{}:{:06.3}", mins, remainder)
    } else {
        format!("{:.3}", remainder)
    }
}

/// Detect laps from the .ld file.
///
/// Tries "Lap Time Running" resets first (sub-sample precision via interpolation),
/// then falls back to "Lap Number" transitions.
pub fn detect_laps(ld: &LdFile) -> Vec<Lap> {
    if let Some(laps) = detect_from_lap_time_running(ld)
        && !laps.is_empty()
    {
        return laps;
    }
    detect_from_lap_number(ld)
}

/// Detect lap boundaries from "Lap Time Running" channel resets.
///
/// The ECU resets this running timer at each beacon (start/finish line) crossing.
/// We interpolate the exact crossing time: `boundary = sample_time - timer_value`.
fn detect_from_lap_time_running(ld: &LdFile) -> Option<Vec<Lap>> {
    let ch = find_channel(ld, &["lap time running", "lap.time.running"])?;
    let data = ld.read_channel_data(ch)?;

    if data.len() < 2 || ch.freq == 0 {
        return None;
    }

    let freq = ch.freq as f64;
    let session_end = data.len() as f64 / freq;

    // Find resets: a large drop in the running timer indicates a beacon crossing
    let mut boundaries = Vec::new();
    for i in 1..data.len() {
        if data[i - 1] - data[i] > 1.0 {
            // Interpolate exact crossing time from the post-reset timer value
            let sample_time = i as f64 / freq;
            let crossing_time = (sample_time - data[i]).max(0.0);
            boundaries.push(crossing_time);
        }
    }

    if boundaries.is_empty() {
        return None;
    }

    Some(boundaries_to_laps(&boundaries, session_end))
}

/// Detect lap boundaries from "Lap Number" channel value transitions.
fn detect_from_lap_number(ld: &LdFile) -> Vec<Lap> {
    let ch = match find_channel(ld, &["lap number", "lap.number"]) {
        Some(ch) => ch,
        None => return vec![],
    };

    let data = match ld.read_channel_data(ch) {
        Some(d) => d,
        None => return vec![],
    };

    if data.is_empty() || ch.freq == 0 {
        return vec![];
    }

    let freq = ch.freq as f64;
    let session_end = data.len() as f64 / freq;

    let mut boundaries = Vec::new();
    let mut prev = data[0] as i64;
    for (i, &val) in data.iter().enumerate().skip(1) {
        let v = val as i64;
        if v != prev {
            boundaries.push(i as f64 / freq);
            prev = v;
        }
    }

    if boundaries.is_empty() {
        // Single lap — the entire session
        return vec![Lap {
            number: 1,
            name: "Lap 1".into(),
            start_time: 0.0,
            end_time: session_end,
        }];
    }

    boundaries_to_laps(&boundaries, session_end)
}

/// Convert a list of boundary times into labeled laps.
///
/// Layout: `[0..b0] = Out Lap`, `[b0..b1] = Lap 1`, …, `[bN..end] = In Lap`.
fn boundaries_to_laps(boundaries: &[f64], session_end: f64) -> Vec<Lap> {
    let mut laps = Vec::with_capacity(boundaries.len() + 1);

    // Out Lap: session start → first boundary
    laps.push(Lap {
        number: 0,
        name: "Out Lap".into(),
        start_time: 0.0,
        end_time: boundaries[0],
    });

    // Timed laps between consecutive boundaries
    for i in 0..boundaries.len() - 1 {
        let lap_num = (i + 1) as u32;
        laps.push(Lap {
            number: lap_num,
            name: format!("Lap {}", lap_num),
            start_time: boundaries[i],
            end_time: boundaries[i + 1],
        });
    }

    // In Lap: last boundary → session end
    let last_num = boundaries.len() as u32;
    laps.push(Lap {
        number: last_num,
        name: "In Lap".into(),
        start_time: *boundaries.last().unwrap(),
        end_time: session_end,
    });

    laps
}

/// Find a channel by name (case-insensitive, underscore/space/dot normalized).
fn find_channel<'a>(ld: &'a LdFile, names: &[&str]) -> Option<&'a ChannelMeta> {
    ld.channels.iter().find(|ch| {
        let normalized = ch.name.to_lowercase().replace(['.', '_'], " ");
        names.iter().any(|n| normalized == *n)
    })
}
