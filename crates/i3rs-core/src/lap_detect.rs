//! Lap boundary detection from channel data.
//!
//! Detects lap boundaries by finding transitions in "Lap Number" or
//! "Lap.Number" channels, which are standard MoTeC M1 ECU outputs.

use crate::ld_parser::{ChannelMeta, LdFile};

/// A detected lap with timing information.
#[derive(Debug, Clone)]
pub struct Lap {
    pub number: u32,
    pub start_time: f64,
    pub end_time: f64,
}

impl Lap {
    pub fn duration(&self) -> f64 {
        self.end_time - self.start_time
    }
}

/// Find the "Lap Number" channel in the file (handles both M1 naming conventions).
fn find_lap_number_channel(ld: &LdFile) -> Option<&ChannelMeta> {
    ld.channels.iter().find(|ch| {
        let name_lower = ch.name.to_lowercase();
        name_lower == "lap number" || name_lower == "lap.number"
    })
}

/// Detect laps from the Lap Number channel in the .ld file.
/// Returns empty vec if no lap channel is found.
pub fn detect_laps(ld: &LdFile) -> Vec<Lap> {
    let ch = match find_lap_number_channel(ld) {
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
    let mut laps = Vec::new();
    let mut current_lap_num = data[0] as u32;
    let mut lap_start_sample: usize = 0;

    for (i, &val) in data.iter().enumerate().skip(1) {
        let lap_num = val as u32;
        if lap_num != current_lap_num {
            // Lap boundary: previous lap ends here
            laps.push(Lap {
                number: current_lap_num,
                start_time: lap_start_sample as f64 / freq,
                end_time: i as f64 / freq,
            });
            current_lap_num = lap_num;
            lap_start_sample = i;
        }
    }

    // Final lap (in progress at end of session)
    laps.push(Lap {
        number: current_lap_num,
        start_time: lap_start_sample as f64 / freq,
        end_time: data.len() as f64 / freq,
    });

    laps
}
