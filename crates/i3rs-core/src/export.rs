//! CSV export for channel data.

use std::io::Write;
use std::path::Path;

/// A channel to export.
pub struct ExportChannel<'a> {
    pub name: &'a str,
    pub data: &'a [f64],
    pub freq: u16,
    pub dec_places: i16,
}

/// Export channels to a CSV file.
///
/// All channels are resampled to the highest frequency via nearest-neighbor.
/// The first column is "Time (s)".
pub fn export_csv(
    path: &Path,
    channels: &[ExportChannel<'_>],
    time_range: Option<(f64, f64)>,
) -> Result<(), String> {
    if channels.is_empty() {
        return Err("no channels to export".into());
    }

    let max_freq = channels.iter().map(|c| c.freq).max().unwrap_or(1).max(1);
    let max_duration = channels
        .iter()
        .filter(|c| c.freq > 0)
        .map(|c| c.data.len() as f64 / c.freq as f64)
        .fold(0.0f64, f64::max);

    let (t_start, t_end) = time_range.unwrap_or((0.0, max_duration));
    let start_sample = (t_start * max_freq as f64).floor() as usize;
    let end_sample = (t_end * max_freq as f64).ceil() as usize;
    let n_rows = end_sample.saturating_sub(start_sample);

    if n_rows == 0 {
        return Err("time range contains no samples".into());
    }

    let file = std::fs::File::create(path).map_err(|e| format!("failed to create file: {}", e))?;
    let mut writer = std::io::BufWriter::new(file);

    // Header
    write!(writer, "Time (s)").map_err(|e| e.to_string())?;
    for ch in channels {
        write!(writer, ",{}", ch.name).map_err(|e| e.to_string())?;
    }
    writeln!(writer).map_err(|e| e.to_string())?;

    // Rows
    let dt = 1.0 / max_freq as f64;
    for i in 0..n_rows {
        let time = (start_sample + i) as f64 * dt;
        write!(writer, "{:.6}", time).map_err(|e| e.to_string())?;

        for ch in channels {
            let src_idx = if ch.freq == max_freq {
                start_sample + i
            } else {
                let t = time;
                (t * ch.freq as f64).round() as usize
            };
            let val = if src_idx < ch.data.len() {
                ch.data[src_idx]
            } else if !ch.data.is_empty() {
                ch.data[ch.data.len() - 1]
            } else {
                0.0
            };
            let dec = ch.dec_places.max(0) as usize;
            write!(writer, ",{:.prec$}", val, prec = dec).map_err(|e| e.to_string())?;
        }
        writeln!(writer).map_err(|e| e.to_string())?;
    }

    writer.flush().map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn export_basic_csv() {
        let dir = std::env::temp_dir();
        let path = dir.join("test_export.csv");

        let ch1 = ExportChannel {
            name: "Speed",
            data: &[10.0, 20.0, 30.0],
            freq: 1,
            dec_places: 1,
        };
        let ch2 = ExportChannel {
            name: "RPM",
            data: &[1000.0, 2000.0, 3000.0],
            freq: 1,
            dec_places: 0,
        };

        export_csv(&path, &[ch1, ch2], None).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines[0], "Time (s),Speed,RPM");
        assert_eq!(lines[1], "0.000000,10.0,1000");
        assert_eq!(lines[2], "1.000000,20.0,2000");
        assert_eq!(lines[3], "2.000000,30.0,3000");
        assert_eq!(lines.len(), 4);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn export_with_time_range() {
        let dir = std::env::temp_dir();
        let path = dir.join("test_export_range.csv");

        let ch = ExportChannel {
            name: "Speed",
            data: &[10.0, 20.0, 30.0, 40.0, 50.0],
            freq: 1,
            dec_places: 0,
        };

        export_csv(&path, &[ch], Some((1.0, 3.0))).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines[0], "Time (s),Speed");
        // Should have 2 rows (samples at t=1 and t=2)
        assert_eq!(lines.len(), 3);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn export_empty_channels() {
        let path = PathBuf::from("/tmp/test_empty.csv");
        let result = export_csv(&path, &[], None);
        assert!(result.is_err());
    }
}
