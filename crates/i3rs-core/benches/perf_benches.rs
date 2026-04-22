use std::collections::HashMap;

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use i3rs_core::{
    ChannelData, FftPlanner, LdFile, TrackData, compute_fft_with_planner, downsample_minmax,
    evaluate_expression_with_aliases, find_nearest_sample,
};

const TEST_LD: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../test_data/VIR_LAP.ld");

fn synthetic_series(len: usize, freq: u16) -> Vec<f64> {
    (0..len)
        .map(|i| {
            let t = i as f64 / freq as f64;
            (t * 3.1).sin() * 20.0 + (t * 0.37).cos() * 5.0 + (i % 97) as f64 * 0.01
        })
        .collect()
}

fn synthetic_track(len: usize, freq: u16) -> TrackData {
    let mut x = Vec::with_capacity(len);
    let mut y = Vec::with_capacity(len);
    let mut time = Vec::with_capacity(len);

    for i in 0..len {
        let t = i as f64 / len as f64 * std::f64::consts::TAU * 6.0;
        let radius = 1.0 + (i as f64 / len as f64) * 0.2;
        x.push(radius * t.cos());
        y.push(radius * t.sin());
        time.push(i as f64 / freq as f64);
    }

    TrackData::from_normalized_parts(x, y, time, freq)
}

fn bench_read_channel_data(c: &mut Criterion) {
    let ld = LdFile::open(TEST_LD).expect("failed to open VIR_LAP.ld");
    let channel = ld
        .channels
        .iter()
        .find(|channel| channel.name == "Engine Speed")
        .expect("Engine Speed channel missing")
        .clone();

    c.bench_function("LdFile::read_channel_data/VIR_LAP/Engine Speed", |b| {
        b.iter(|| black_box(ld.read_channel_data(black_box(&channel)).unwrap()))
    });
}

fn bench_downsample_minmax(c: &mut Criterion) {
    let samples = synthetic_series(1_000_000, 200);
    c.bench_function("downsample_minmax/synthetic_1m_to_2k", |b| {
        b.iter(|| black_box(downsample_minmax(black_box(&samples), 200, 0, 2_048)))
    });
}

fn bench_evaluate_expression(c: &mut Criterion) {
    let mut channels = HashMap::new();
    channels.insert(
        "Engine Speed".to_string(),
        ChannelData {
            samples: synthetic_series(400_000, 100),
            freq: 100,
        },
    );
    channels.insert(
        "Vehicle Speed".to_string(),
        ChannelData {
            samples: synthetic_series(200_000, 50),
            freq: 50,
        },
    );
    channels.insert(
        "Throttle Position".to_string(),
        ChannelData {
            samples: synthetic_series(200_000, 50),
            freq: 50,
        },
    );

    let aliases = HashMap::from([
        ("RPM".to_string(), "Engine Speed".to_string()),
        ("Speed".to_string(), "Vehicle Speed".to_string()),
    ]);
    let expression = "smooth(RPM, 7) + derivative(Speed) + Throttle_Position * 0.1";

    c.bench_function("evaluate_expression_with_aliases/synthetic", |b| {
        b.iter(|| {
            black_box(
                evaluate_expression_with_aliases(
                    black_box(expression),
                    black_box(&channels),
                    black_box(&aliases),
                )
                .unwrap(),
            )
        })
    });
}

fn bench_fft(c: &mut Criterion) {
    let samples = synthetic_series(32_768, 512);
    let mut planner = FftPlanner::new();

    c.bench_function("compute_fft_with_planner/synthetic_32k", |b| {
        b.iter(|| {
            black_box(compute_fft_with_planner(
                black_box(&samples),
                512.0,
                black_box(&mut planner),
            ))
        })
    });
}

fn bench_find_nearest_sample(c: &mut Criterion) {
    let track = synthetic_track(200_000, 20);
    c.bench_function("find_nearest_sample/synthetic_200k", |b| {
        b.iter(|| black_box(find_nearest_sample(black_box(&track), 0.52, -0.41)))
    });
}

criterion_group!(
    perf_benches,
    bench_read_channel_data,
    bench_downsample_minmax,
    bench_evaluate_expression,
    bench_fft,
    bench_find_nearest_sample
);
criterion_main!(perf_benches);
