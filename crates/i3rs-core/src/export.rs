//! CSV export for channel data.

use std::io::Write;
use std::path::Path;

/// A channel to export.
pub struct ExportChannel<'a> {
    pub name: &'a str,
    /// Unit of the exported values (empty for unitless channels).
    pub unit: &'a str,
    /// Source samples; the display transform is applied when writing each cell.
    pub data: &'a [f64],
    pub freq: f64,
    pub dec_places: i16,
    pub display_scale: f64,
    pub display_offset: f64,
}

struct ExportResamplePlan<'a> {
    data: &'a [f64],
    dec_places: usize,
    display_scale: f64,
    display_offset: f64,
    source_indices: Vec<usize>,
}

fn write_header_cell(writer: &mut impl Write, value: &str) -> std::io::Result<()> {
    if value.contains([',', '"', '\n', '\r']) {
        write!(writer, "\"{}\"", value.replace('"', "\"\""))
    } else {
        write!(writer, "{value}")
    }
}

/// Export channels to a CSV file.
///
/// All channels are resampled to the highest frequency via nearest-neighbor.
/// The first column is "Time (s)".
/// Values use `raw * display_scale + display_offset` and the supplied precision.
/// Headers are `Name (unit)`, or just `Name` for unitless channels. Missing or
/// non-finite values produce empty cells, including non-finite transformed values.
pub fn export_csv(
    path: &Path,
    channels: &[ExportChannel<'_>],
    time_range: Option<(f64, f64)>,
) -> Result<(), String> {
    if channels.is_empty() {
        return Err("no channels to export".into());
    }

    let max_freq = channels.iter().map(|c| c.freq).fold(1.0_f64, f64::max);
    let max_duration = channels
        .iter()
        .filter(|c| c.freq > 0.0)
        .map(|c| c.data.len() as f64 / c.freq)
        .fold(0.0f64, f64::max);

    let (t_start, t_end) = time_range.unwrap_or((0.0, max_duration));
    let start_sample = (t_start * max_freq).floor() as usize;
    let end_sample = (t_end * max_freq).ceil() as usize;
    let n_rows = end_sample.saturating_sub(start_sample);

    if n_rows == 0 {
        return Err("time range contains no samples".into());
    }

    let file = std::fs::File::create(path).map_err(|e| format!("failed to create file: {}", e))?;
    let mut writer = std::io::BufWriter::new(file);

    // Header
    write!(writer, "Time (s)").map_err(|e| e.to_string())?;
    for ch in channels {
        write!(writer, ",").map_err(|e| e.to_string())?;
        let header = if ch.unit.is_empty() {
            ch.name.to_string()
        } else {
            format!("{} ({})", ch.name, ch.unit)
        };
        write_header_cell(&mut writer, &header).map_err(|e| e.to_string())?;
    }
    writeln!(writer).map_err(|e| e.to_string())?;

    let channel_plans: Vec<ExportResamplePlan<'_>> = channels
        .iter()
        .map(|channel| {
            let dec_places = channel.dec_places.max(0) as usize;
            let source_indices = if channel.freq == max_freq {
                (0..n_rows).map(|row| start_sample + row).collect()
            } else {
                let ratio = channel.freq / max_freq;
                (0..n_rows)
                    .map(|row| ((start_sample + row) as f64 * ratio).round() as usize)
                    .collect()
            };
            ExportResamplePlan {
                data: channel.data,
                dec_places,
                display_scale: channel.display_scale,
                display_offset: channel.display_offset,
                source_indices,
            }
        })
        .collect();

    // Rows
    let dt = 1.0 / max_freq;
    for row in 0..n_rows {
        let time = (start_sample + row) as f64 * dt;
        write!(writer, "{:.6}", time).map_err(|e| e.to_string())?;

        for plan in &channel_plans {
            let src_idx = plan.source_indices[row];
            let value = plan
                .data
                .get(src_idx)
                .copied()
                .filter(|v| v.is_finite())
                .map(|v| v * plan.display_scale + plan.display_offset)
                .filter(|v| v.is_finite());
            if let Some(val) = value {
                write!(writer, ",{:.prec$}", val, prec = plan.dec_places)
                    .map_err(|e| e.to_string())?;
            } else {
                write!(writer, ",").map_err(|e| e.to_string())?;
            }
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
            unit: "km/h",
            display_scale: 1.0,
            display_offset: 0.0,
            data: &[10.0, 20.0, 30.0],
            freq: 1.0,
            dec_places: 1,
        };
        let ch2 = ExportChannel {
            name: "RPM",
            unit: "rpm",
            display_scale: 1.0,
            display_offset: 0.0,
            data: &[1000.0, 2000.0, 3000.0],
            freq: 1.0,
            dec_places: 0,
        };

        export_csv(&path, &[ch1, ch2], None).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines[0], "Time (s),Speed (km/h),RPM (rpm)");
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
            unit: "km/h",
            display_scale: 1.0,
            display_offset: 0.0,
            data: &[10.0, 20.0, 30.0, 40.0, 50.0],
            freq: 1.0,
            dec_places: 0,
        };

        export_csv(&path, &[ch], Some((1.0, 3.0))).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines[0], "Time (s),Speed (km/h)");
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

    #[test]
    fn export_display_units_scale_offset_and_precision_with_mixed_rates() {
        let path = std::env::temp_dir().join("i3rs_export_display_units.csv");
        let channels = [
            ExportChannel {
                name: "Pressure",
                unit: "psi",
                data: &[100.0, 200.0],
                freq: 2.0,
                dec_places: 3,
                display_scale: 0.145_037_738,
                display_offset: 0.0,
            },
            ExportChannel {
                name: "Temperature",
                unit: "°F",
                data: &[0.0, 100.0],
                freq: 1.0,
                dec_places: 1,
                display_scale: 1.8,
                display_offset: 32.0,
            },
        ];
        export_csv(&path, &channels, Some((0.0, 1.0))).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "Time (s),Pressure (psi),Temperature (°F)\n0.000000,14.504,32.0\n0.500000,29.008,212.0\n"
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn export_escapes_headers_and_leaves_nonfinite_transformed_cells_empty() {
        let path = std::env::temp_dir().join("i3rs_export_escaped_headers.csv");
        let channels = [
            ExportChannel {
                name: "Pressure, \"left\"",
                unit: "psi",
                data: &[1.0, f64::MAX, f64::INFINITY, f64::NAN],
                freq: 1.0,
                dec_places: 0,
                display_scale: -2.0,
                display_offset: 10.0,
            },
            ExportChannel {
                name: "State",
                unit: "",
                data: &[1.0],
                freq: 1.0,
                dec_places: 0,
                display_scale: 1.0,
                display_offset: 0.0,
            },
        ];
        export_csv(&path, &channels, None).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "Time (s),\"Pressure, \"\"left\"\" (psi)\",State\n0.000000,8,1\n1.000000,,\n2.000000,,\n3.000000,,\n"
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn missing_samples_export_as_empty_cells() {
        let path = std::env::temp_dir().join("i3rs_missing_samples.csv");
        let channel = ExportChannel {
            name: "Lambda",
            unit: "",
            display_scale: 1.0,
            display_offset: 0.0,
            data: &[0.895, f64::NAN, 1.020],
            freq: 1.0,
            dec_places: 3,
        };
        export_csv(&path, &[channel], Some((0.0, 4.0))).unwrap();
        let csv = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            csv,
            "Time (s),Lambda\n0.000000,0.895\n1.000000,\n2.000000,1.020\n3.000000,\n"
        );
        std::fs::remove_file(path).unwrap();
    }
}
