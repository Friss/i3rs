//! Display-only reduction for very dense overview traces.
//!
//! Analysis and cursor reads continue to use the original channel arrays.
use egui_plot::PlotPoint;

// Ordinary lap views stay at native resolution even if several samples share a
// pixel. Only large, densely packed traces need to bound drawing work.
const NATIVE_POINT_LIMIT: usize = 100_000;
const POINTS_PER_PIXEL_LIMIT: usize = 8;

/// Keep actual first/min/max/last samples per physical pixel column, in original
/// order. Never average values or move extrema to a synthetic bucket timestamp.
/// Invalid runs stay as line breaks. Offscreen edge points retain their columns.
pub(super) fn display_trace_points(
    points: Vec<PlotPoint>,
    x_min: f64,
    x_max: f64,
    pixel_columns: usize,
) -> Vec<PlotPoint> {
    let span = x_max - x_min;
    if points.len() <= NATIVE_POINT_LIMIT
        || points.len() <= pixel_columns.saturating_mul(POINTS_PER_PIXEL_LIMIT)
        || pixel_columns == 0
        || !span.is_finite()
        || span <= 0.0
    {
        return points;
    }
    let columns_per_x = pixel_columns as f64 / span;
    let column = |point: &PlotPoint| ((point.x - x_min) * columns_per_x).floor() as i64;
    let valid = |point: &PlotPoint| point.x.is_finite() && point.y.is_finite();
    let mut output = Vec::with_capacity(pixel_columns.saturating_mul(4).min(points.len()));
    let mut start = 0;
    while start < points.len() {
        if !valid(&points[start]) {
            output.push(points[start]);
            start += 1;
            while start < points.len() && !valid(&points[start]) {
                start += 1;
            }
            continue;
        }
        let bucket = column(&points[start]);
        let (mut min, mut max) = (start, start);
        let mut end = start + 1;
        while end < points.len() && valid(&points[end]) && column(&points[end]) == bucket {
            if points[end].y < points[min].y {
                min = end;
            }
            if points[end].y > points[max].y {
                max = end;
            }
            end += 1;
        }
        if end - start <= 4 {
            output.extend_from_slice(&points[start..end]);
        } else {
            let mut indices = [start, min, max, end - 1];
            indices.sort_unstable();
            let mut previous = None;
            for idx in indices {
                if previous != Some(idx) {
                    output.push(points[idx]);
                }
                previous = Some(idx);
            }
        }
        start = end;
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lap_sized_views_keep_every_sample() {
        let points: Vec<_> = (0..20_000)
            .map(|i| PlotPoint::new(i as f64 / 100., (i % 7) as f64))
            .collect();
        let copy = points.clone();
        assert_eq!(display_trace_points(points, 0., 200., 100), copy);
    }

    #[test]
    fn dense_extrema_keep_original_time_order_and_bucket_endpoints() {
        let mut points: Vec<_> = (0..200_000).map(|i| PlotPoint::new(i as f64, 0.)).collect();
        // A maximum BEFORE a minimum must never be reordered into min/max order.
        points[123].y = 100.;
        points[456].y = -90.;
        let reduced = display_trace_points(points, 0., 200_000., 100);
        assert_eq!(
            &reduced[..4],
            &[
                PlotPoint::new(0., 0.),
                PlotPoint::new(123., 100.),
                PlotPoint::new(456., -90.),
                PlotPoint::new(1999., 0.),
            ]
        );
        assert_eq!(reduced.last().unwrap().x, 199_999.);
        assert!(reduced.len() <= 400);
    }

    #[test]
    fn gaps_split_buckets_and_do_not_hide_short_events() {
        let mut points: Vec<_> = (0..200_000).map(|i| PlotPoint::new(i as f64, 0.)).collect();
        points[8].y = f64::NAN;
        points[9].y = f64::NAN;
        points[10].y = 1.;
        points[11].y = f64::NAN;
        let reduced = display_trace_points(points, 0., 200_000., 100);
        let event = reduced.iter().position(|p| p.x == 10.).unwrap();
        assert!(reduced[event - 1].y.is_nan());
        assert_eq!(reduced[event].y, 1.);
        assert!(reduced[event + 1].y.is_nan());
        assert_eq!(reduced.iter().filter(|p| p.y.is_nan()).count(), 2);
    }

    #[test]
    fn each_pixel_keeps_its_envelope_with_exact_source_samples() {
        let points: Vec<_> = (0..200_000)
            .map(|i| PlotPoint::new(i as f64, ((i * 73) % 997) as f64))
            .collect();
        let reduced = display_trace_points(points.clone(), 0., 200_000., 100);
        for p in &reduced {
            assert_eq!(*p, points[p.x as usize]);
        }
        for bucket in 0..100 {
            let start = bucket * 2000;
            let original = &points[start..start + 2000];
            let selected: Vec<_> = reduced
                .iter()
                .filter(|p| p.x >= start as f64 && p.x < (start + 2000) as f64)
                .collect();
            assert_eq!(selected.first().unwrap().x, original[0].x);
            assert_eq!(selected.last().unwrap().x, original.last().unwrap().x);
            assert_eq!(
                selected.iter().map(|p| p.y).fold(f64::INFINITY, f64::min),
                original.iter().map(|p| p.y).fold(f64::INFINITY, f64::min)
            );
            assert_eq!(
                selected
                    .iter()
                    .map(|p| p.y)
                    .fold(f64::NEG_INFINITY, f64::max),
                original
                    .iter()
                    .map(|p| p.y)
                    .fold(f64::NEG_INFINITY, f64::max)
            );
        }
    }
}
