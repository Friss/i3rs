//! Lap boundary detection from channel data.
//!
//! Detects recorded beacon events, with running-timer and lap-number fallbacks.
//! Labels laps as Out Lap,
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
/// Recorded beacon events take priority over ECU timers, which can reset when
/// the engine stops and can miss two crossings between consecutive samples.
pub fn detect_laps(ld: &LdFile) -> Vec<Lap> {
    if let Some(ch) = find_channel(ld, &["beacon (internal)"])
        && ch.data_type == crate::ld_parser::DataType::Int16
        && let Some(data) = ld.read_channel_data(ch)
    {
        let boundaries = beacon_boundaries(&data, ch.freq, ld.duration_secs());
        if !boundaries.is_empty() {
            return boundaries_to_laps(&boundaries, ld.duration_secs());
        }
    }
    if let Some(laps) = detect_from_lap_time_running(ld)
        && !laps.is_empty()
    {
        return laps;
    }
    detect_from_lap_number(ld)
}

/// Internal beacons are three tagged 16-bit words (11, 10, 01 high bits).
/// The third word records milliseconds within the second. The first word is
/// stored in the sample immediately after the event's time bucket. This also
/// works for cropped logs, whose packed ECU tick counter keeps its old origin.
fn beacon_boundaries(data: &[f64], freq: u16, session_end: f64) -> Vec<f64> {
    if freq == 0 {
        return Vec::new();
    }
    let mut boundaries = Vec::new();
    for (i, event) in data.windows(3).enumerate().skip(1) {
        if event.iter().any(|v| !v.is_finite()) {
            continue;
        }
        let words = [event[0], event[1], event[2]].map(|v| v as i16 as u16);
        if words[0] & 0xc000 != 0xc000 || words[1] & 0xc000 != 0x8000 || words[2] & 0xc000 != 0x4000
        {
            continue;
        }
        let millis = words[2] & 0x3fff;
        if millis >= 1000 {
            continue;
        }
        let lower = (i - 1) as f64 / freq as f64;
        let upper = i as f64 / freq as f64;
        let fraction = millis as f64 / 1000.0;
        // Allow half a millisecond for quantization of the event timestamp.
        let time = (lower - fraction - 0.0005).ceil() + fraction;
        if time >= 0.0
            && time <= upper + 0.0005
            && time < session_end
            && boundaries.last().is_none_or(|previous| time > *previous)
        {
            boundaries.push(time);
        }
    }
    boundaries
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
    let session_end = ld.duration_secs();

    // Find resets: a large drop in the running timer indicates a beacon crossing
    let mut boundaries = Vec::new();
    for i in 1..data.len() {
        if data[i - 1] - data[i] > 1.0 {
            // Interpolate exact crossing time from the post-reset timer value
            let sample_time = i as f64 / freq;
            let crossing_time = (sample_time - data[i]).max(0.0);
            if crossing_time < session_end
                && boundaries
                    .last()
                    .is_none_or(|previous| crossing_time > *previous)
            {
                boundaries.push(crossing_time);
            }
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
    let session_end = ld.duration_secs();

    let mut boundaries = Vec::new();
    let mut prev = None;
    for (i, &val) in data.iter().enumerate() {
        if !val.is_finite() {
            continue;
        }
        let v = val as i64;
        if prev.is_some_and(|previous| v != previous) && (i as f64 / freq) < session_end {
            boundaries.push(i as f64 / freq);
        }
        prev = Some(v);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn put_beacon(data: &mut [f64], i: usize, millis: u16) {
        data[i] = 0xc123_u16 as i16 as f64;
        data[i + 1] = 0x8000_u16 as i16 as f64;
        data[i + 2] = (0x4000 | millis) as f64;
    }

    #[test]
    fn tagged_beacons_keep_close_crossings_and_millisecond_boundaries() {
        let mut data = vec![6100.0; 200];
        // 68 Hz stream: the record starts in the bucket after the event.
        put_beacon(&mut data, 36, 515);
        put_beacon(&mut data, 40, 574);
        put_beacon(&mut data, 68, 999);
        assert_eq!(beacon_boundaries(&data, 68, 3.0), vec![0.515, 0.574, 0.999]);
    }

    #[test]
    fn cropped_one_hz_log_uses_local_stream_time_and_rejects_bad_tags() {
        let mut data = vec![6100.0; 140];
        put_beacon(&mut data, 1, 101);
        put_beacon(&mut data, 129, 493);
        put_beacon(&mut data, 135, 1001);
        assert_eq!(beacon_boundaries(&data, 1, 140.0), vec![0.101, 128.493]);
        data[2] = 0.0;
        assert_eq!(beacon_boundaries(&data, 1, 140.0), vec![128.493]);
    }
}
