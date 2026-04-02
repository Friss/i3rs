//! CLI tool for parsing MoTeC .ld files using i3rs-core.

use i3rs_core::LdFile;
use std::collections::BTreeMap;
use std::env;
use std::process;

fn fmt_val(v: Option<f64>) -> String {
    match v {
        None => "            ".to_string(),
        Some(v) if v.abs() < 0.01 && v != 0.0 => format!("{:12.4e}", v),
        Some(v) => format!("{:12.4}", v),
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <file.ld>", args[0]);
        process::exit(1);
    }

    let filepath = &args[1];
    let ld = match LdFile::open(filepath) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Error: {}", e);
            process::exit(1);
        }
    };

    let sep = "-".repeat(100);
    let s = &ld.session;
    let e = &ld.event;

    println!("\nParsing: {}", filepath);
    println!("Size   : {} bytes\n", ld.file_size());

    println!("{}", sep);
    println!("  MoTeC i2 Log File Summary");
    println!("{}", sep);
    println!("  Date/Time     : {}  {}", s.date, s.time);
    println!("  Driver        : {}", s.driver);
    println!("  Vehicle ID    : {}", s.vehicle_id);
    println!("  Venue         : {}", s.venue);
    println!("  Comment       : {}", s.short_comment);
    println!(
        "  Device        : {} (serial {}, v{})",
        s.device_type, s.device_serial, s.device_version
    );
    println!(
        "  Channels      : {} (header), {} (parsed)",
        s.num_channels_header,
        ld.channels.len()
    );

    if !e.event_name.is_empty() {
        println!("\n  Event         : {}", e.event_name);
    }
    if !e.session.is_empty() {
        println!("  Session       : {}", e.session);
    }
    if !e.venue_detail.is_empty() {
        println!("  Venue Detail  : {}", e.venue_detail);
    }
    if !e.vehicle_id.is_empty() {
        println!(
            "  Vehicle       : {} (type={}, weight={})",
            e.vehicle_id, e.vehicle_type, e.vehicle_weight
        );
    }

    let duration = ld.duration_secs();
    if duration > 0.0 {
        let mins = (duration / 60.0) as u32;
        let secs = duration - (mins as f64 * 60.0);
        println!(
            "\n  Est. Duration : {}m {:.1}s  ({:.1}s)",
            mins, secs, duration
        );
    }

    println!("\n{}", sep);
    println!(
        "  {:>3}  {:<45} {:<8} {:>5} {:>8} {:<9} {:>12} {:>12} {:>12}",
        "#", "Channel Name", "Unit", "Hz", "Samples", "Type", "Min", "Max", "Mean"
    );
    println!("{}", sep);

    for ch in &ld.channels {
        // Read data on-demand to compute stats
        let (min, max, mean) = if let Some(data) = ld.read_channel_data(ch) {
            if data.is_empty() {
                (None, None, None)
            } else {
                let mut min_v = data[0];
                let mut max_v = data[0];
                let mut sum = 0.0;
                for &v in &data {
                    if v < min_v {
                        min_v = v;
                    }
                    if v > max_v {
                        max_v = v;
                    }
                    sum += v;
                }
                (Some(min_v), Some(max_v), Some(sum / data.len() as f64))
            }
        } else {
            (None, None, None)
        };

        println!(
            "  {:3}  {:<45} {:<8} {:5} {:8} {:<9} {} {} {}",
            ch.index,
            ch.name,
            ch.unit,
            ch.freq,
            ch.n_data,
            ch.data_type.name(),
            fmt_val(min),
            fmt_val(max),
            fmt_val(mean)
        );
    }

    println!("{}", sep);
    println!("  Total channels: {}", ld.channels.len());

    let mut freq_counts: BTreeMap<u16, usize> = BTreeMap::new();
    for ch in &ld.channels {
        *freq_counts.entry(ch.freq).or_insert(0) += 1;
    }
    let parts: Vec<String> = freq_counts
        .iter()
        .map(|(f, c)| format!("{} ch @ {} Hz", c, f))
        .collect();
    println!("  By sample rate: {}", parts.join(", "));
    println!("{}", sep);
}
