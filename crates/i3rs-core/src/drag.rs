//! Drag run details and reusable distance-milestone calculations.
use crate::ranges::{AnalysisRange, RangeKind};
use serde::{Deserialize, Serialize};

pub const MILESTONES_FT: [u32; 5] = [60, 330, 660, 1000, 1320];

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunDetails {
    /// Cumulative elapsed seconds from the run start, excluding reaction time.
    pub elapsed: [Option<f64>; 5],
}
impl RunDetails {
    pub fn validate(&self, duration: f64) -> Result<(), String> {
        let mut previous = 0.;
        for value in self.elapsed.iter().flatten() {
            if !value.is_finite() || *value <= previous || *value > duration + 1e-6 {
                return Err("Time-slip elapsed times must increase and fit within the run".into());
            }
            previous = *value;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DragSetup {
    pub engine: String,
    pub drive_shaft: String,
    pub gear: String,
    /// Transmission input/output ratios, indexed by forward gear minus one.
    pub ratios: Vec<f64>,
    pub generate_input_shaft: bool,
    pub generate_clutch_slip: bool,
}
impl Default for DragSetup {
    fn default() -> Self {
        Self {
            engine: "Engine Speed".into(),
            drive_shaft: "Drive Shaft Speed".into(),
            gear: "Gear".into(),
            ratios: Vec::new(),
            generate_input_shaft: true,
            generate_clutch_slip: true,
        }
    }
}
impl DragSetup {
    pub fn validate(&self) -> Result<(), String> {
        if self.ratios.len() > 16 || self.ratios.iter().any(|r| !r.is_finite() || *r <= 0.) {
            return Err("Gear ratios must be finite and positive (at most 16 gears)".into());
        }
        for name in [&self.engine, &self.drive_shaft, &self.gear] {
            if name.trim().is_empty() || name.contains(['\'', '\n', '\r']) {
                return Err("Drag inputs need valid channel references without quotes".into());
            }
        }
        Ok(())
    }
    /// Use the shared expression engine: neutral, unknown gear and zero engine
    /// speed produce missing values, rather than a plausible but false slip.
    pub fn expressions(&self) -> Result<Vec<(&'static str, String, &'static str)>, String> {
        self.validate()?;
        if self.ratios.is_empty() {
            return Err("Configure transmission gear ratios to calculate clutch slip".into());
        }
        let mut ratio = "(0/0)".to_string();
        for (i, value) in self.ratios.iter().enumerate().rev() {
            ratio = format!("choose('{}' == {}, {}, {})", self.gear, i + 1, value, ratio);
        }
        let input = format!("'{}'[rpm] * ({ratio})", self.drive_shaft);
        let mut result = Vec::new();
        if self.generate_input_shaft {
            result.push(("Input Shaft Speed", input.clone(), "rpm"));
        }
        if self.generate_clutch_slip {
            result.push((
                "Clutch Slip",
                format!(
                    "choose('{}'[rpm] > 0, (1 - ({input}) / '{}'[rpm]) * 100, 0/0)",
                    self.engine, self.engine
                ),
                "%",
            ));
        }
        Ok(result)
    }
}

/// Cumulative crossing times. Official slip values take precedence individually;
/// remaining crossings require contiguous finite, nondecreasing distance data.
/// Never interpolate across missing data or across another run's boundary.
pub fn milestone_times(
    run: &AnalysisRange,
    slip: Option<&RunDetails>,
    distance: Option<(&[f64], f64)>,
) -> [Option<f64>; 5] {
    if run.kind != RangeKind::Run
        || !run.start.is_finite()
        || run.start < 0.
        || !run.end.is_finite()
        || run.end <= run.start
    {
        return [None; 5];
    }
    let official = slip
        .filter(|s| s.validate(run.end - run.start).is_ok())
        .map_or([None; 5], |s| s.elapsed);
    let mut result = official;
    if let Some((data, hz)) = distance.filter(|(_, hz)| *hz > 0.0)
        && let Some(origin) = crate::sample_at_time(data, hz, run.start)
    {
        let mut previous = (run.start, origin);
        let first = (run.start * hz).floor() as usize + 1;
        for (i, &d) in data.iter().enumerate().skip(first) {
            let t = i as f64 / hz;
            let (t, d) = if t > run.end {
                let Some(d) = crate::sample_at_time(data, hz, run.end) else {
                    break;
                };
                (run.end, d)
            } else {
                (t, d)
            };
            if !d.is_finite() || d < previous.1 {
                break;
            }
            for (m, ft) in MILESTONES_FT.iter().enumerate() {
                let target = origin + *ft as f64 * 0.3048;
                if result[m].is_none() && previous.1 < target && target <= d {
                    result[m] = Some(
                        previous.0 + (target - previous.1) / (d - previous.1) * (t - previous.0)
                            - run.start,
                    );
                }
            }
            previous = (t, d);
            if t >= run.end {
                break;
            }
        }
    }
    // Mixed official/computed times must still describe one ordered run.
    for (i, value) in result.iter_mut().enumerate() {
        if official[i].is_none()
            && let Some(t) = *value
        {
            let before = official[..i].iter().flatten().last().copied().unwrap_or(0.);
            let after = official[i + 1..]
                .iter()
                .flatten()
                .next()
                .copied()
                .unwrap_or(run.end - run.start + 1e-6);
            if t <= before || t >= after {
                *value = None;
            }
        }
    }
    let mut previous = 0.;
    for value in &mut result {
        if let Some(t) = *value {
            if !t.is_finite() || t <= previous || t > run.end - run.start + 1e-6 {
                *value = None;
            } else {
                previous = t;
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn run() -> AnalysisRange {
        AnalysisRange {
            id: "run".into(),
            name: "Q1".into(),
            start: 0.25,
            end: 10.25,
            kind: RangeKind::Run,
            trust: Default::default(),
            comment: String::new(),
            reaction_time: Some(0.4),
        }
    }
    #[test]
    fn milestones_interpolate_off_grid_and_do_not_include_reaction_or_bridge_gaps() {
        let data: Vec<_> = (0..12).map(|i| i as f64 * 60.96).collect();
        let times = milestone_times(&run(), None, Some((&data, 1.0)));
        for (t, ft) in times.into_iter().zip(MILESTONES_FT) {
            assert!((t.unwrap() - ft as f64 / 200.).abs() < 1e-10);
        }
        let mut gap = data;
        gap[2] = f64::NAN;
        let times = milestone_times(&run(), None, Some((&gap, 1.0)));
        assert!(times[0].is_some());
        assert!(times[1..].iter().all(Option::is_none));
        let mut stage = run();
        stage.kind = RangeKind::Stage;
        assert_eq!(milestone_times(&stage, None, Some((&gap, 1.0))), [None; 5]);
        let slip = RunDetails {
            elapsed: [Some(1.2), Some(2.81), Some(4.07), Some(5.16), Some(6.138)],
        };
        assert_eq!(milestone_times(&run(), Some(&slip), None), slip.elapsed);
        assert!(slip.validate(6.138).is_ok());
        assert!(slip.validate(5.).is_err());
        let mixed = RunDetails {
            elapsed: [None, Some(0.2), None, None, None],
        };
        let data: Vec<_> = (0..12).map(|i| i as f64 * 60.96).collect();
        let times = milestone_times(&run(), Some(&mixed), Some((&data, 1.0)));
        assert_eq!(times[0], None);
        assert_eq!(times[1], Some(0.2));
    }
    #[test]
    fn transmission_and_clutch_expressions_use_shared_engine_and_missing_gear_contract() {
        use crate::ChannelData;
        let setup = DragSetup {
            ratios: vec![2., 1.],
            ..Default::default()
        };
        let data = std::collections::HashMap::from([
            (
                "Engine Speed".into(),
                ChannelData {
                    samples: vec![4000., 4000., 4000., 0.],
                    freq: 1.0,
                },
            ),
            (
                "Drive Shaft Speed".into(),
                ChannelData {
                    samples: vec![1000.; 4],
                    freq: 1.0,
                },
            ),
            (
                "Gear".into(),
                ChannelData {
                    samples: vec![1., 2., 0., 1.],
                    freq: 1.0,
                },
            ),
        ]);
        let options = crate::EvaluationOptions {
            input_units: std::collections::BTreeMap::from([
                ("Engine Speed".into(), "rpm".into()),
                ("Drive Shaft Speed".into(), "rpm".into()),
            ]),
            ..Default::default()
        };
        let expr = setup.expressions().unwrap();
        let (shaft, _) = crate::evaluate_expression_with_options(
            &expr[0].1,
            &data,
            &Default::default(),
            &options,
        )
        .unwrap();
        let (slip, _) = crate::evaluate_expression_with_options(
            &expr[1].1,
            &data,
            &Default::default(),
            &options,
        )
        .unwrap();
        assert_eq!(&shaft[..2], &[2000., 1000.]);
        assert_eq!(&slip[..2], &[50., 75.]);
        assert!(slip[2].is_nan() && slip[3].is_nan());
        assert!(DragSetup::default().expressions().is_err());
    }
}
