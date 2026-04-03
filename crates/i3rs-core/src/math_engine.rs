//! Math channel evaluator: evaluates parsed expressions against channel data.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;
use std::sync::LazyLock;

use crate::math_expr::{BinOp, Expr, parse_expression, referenced_channels};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Channel data provided to the evaluator.
#[derive(Clone)]
pub struct ChannelData {
    pub samples: Vec<f64>,
    pub freq: u16,
}

/// Error from math evaluation.
#[derive(Debug, Clone, PartialEq)]
pub struct MathError {
    pub message: String,
}

impl fmt::Display for MathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "math error: {}", self.message)
    }
}

impl std::error::Error for MathError {}

// ---------------------------------------------------------------------------
// Channel name resolution
// ---------------------------------------------------------------------------

/// Resolve a channel reference against available channel names.
///
/// Resolution priority: exact → underscore-to-space → underscore-to-dot
/// → alias (exact + normalized) → case-insensitive channel → case-insensitive alias.
fn resolve_channel_name<'a>(
    reference: &str,
    available: &'a HashMap<String, ChannelData>,
    aliases: &HashMap<String, String>,
) -> Option<&'a str> {
    if let Some((k, _)) = available.get_key_value(reference) {
        return Some(k);
    }

    let with_spaces = reference.replace('_', " ");
    if let Some((k, _)) = available.get_key_value(&with_spaces) {
        return Some(k);
    }

    let with_dots = reference.replace('_', ".");
    if let Some((k, _)) = available.get_key_value(&with_dots) {
        return Some(k);
    }

    for variant in [reference, with_spaces.as_str(), with_dots.as_str()] {
        if let Some(target) = aliases.get(variant) {
            if let Some((k, _)) = available.get_key_value(target) {
                return Some(k);
            }
        }
    }

    for key in available.keys() {
        if key.eq_ignore_ascii_case(reference) {
            return Some(key);
        }
    }

    let ref_lower = reference.to_ascii_lowercase();
    for (alias, target) in aliases {
        if alias.to_ascii_lowercase() == ref_lower {
            if let Some((k, _)) = available.get_key_value(target) {
                return Some(k);
            }
        }
    }

    None
}

/// Resolve an alias reference to its target channel name.
/// Used by the app to determine which physical channels to load before evaluation.
pub fn resolve_alias_target(reference: &str, aliases: &HashMap<String, String>) -> Option<String> {
    if let Some(target) = aliases.get(reference) {
        return Some(target.clone());
    }
    let with_spaces = reference.replace('_', " ");
    if let Some(target) = aliases.get(&with_spaces) {
        return Some(target.clone());
    }
    let with_dots = reference.replace('_', ".");
    if let Some(target) = aliases.get(&with_dots) {
        return Some(target.clone());
    }
    let ref_lower = reference.to_ascii_lowercase();
    for (alias, target) in aliases {
        if alias.to_ascii_lowercase() == ref_lower {
            return Some(target.clone());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Resampling
// ---------------------------------------------------------------------------

/// Resample a channel to a target frequency using linear interpolation.
/// Returns a borrowed slice when no resampling is needed.
fn resample<'a>(data: &'a [f64], src_freq: u16, target_freq: u16, target_len: usize) -> Cow<'a, [f64]> {
    if src_freq == target_freq && data.len() == target_len {
        return Cow::Borrowed(data);
    }
    if data.is_empty() {
        return Cow::Owned(vec![0.0; target_len]);
    }

    let mut out = Vec::with_capacity(target_len);
    let ratio = src_freq as f64 / target_freq as f64;

    for i in 0..target_len {
        let src_idx = i as f64 * ratio;
        let lo = src_idx.floor() as usize;
        let hi = lo + 1;
        let frac = src_idx - lo as f64;

        let val = if hi >= data.len() {
            data[data.len() - 1]
        } else {
            data[lo] * (1.0 - frac) + data[hi] * frac
        };
        out.push(val);
    }
    Cow::Owned(out)
}

// ---------------------------------------------------------------------------
// Evaluator
// ---------------------------------------------------------------------------

static EMPTY_ALIASES: LazyLock<HashMap<String, String>> = LazyLock::new(HashMap::new);

/// Evaluate a parsed expression against channel data (no aliases).
pub fn evaluate(
    expr: &Expr,
    channels: &HashMap<String, ChannelData>,
    output_freq: u16,
    output_len: usize,
) -> Result<Vec<f64>, MathError> {
    eval_impl(expr, channels, output_freq, output_len, &EMPTY_ALIASES)
}

fn eval_impl(
    expr: &Expr,
    channels: &HashMap<String, ChannelData>,
    output_freq: u16,
    output_len: usize,
    aliases: &HashMap<String, String>,
) -> Result<Vec<f64>, MathError> {
    match expr {
        Expr::Number(n) => Ok(vec![*n; output_len]),

        Expr::Channel(name) => {
            let resolved =
                resolve_channel_name(name, channels, aliases).ok_or_else(|| {
                    MathError {
                        message: format!("unknown channel '{}'", name),
                    }
                })?;
            let ch = &channels[resolved];
            Ok(resample(&ch.samples, ch.freq, output_freq, output_len).into_owned())
        }

        Expr::UnaryNeg(inner) => {
            let vals = eval_impl(inner, channels, output_freq, output_len, aliases)?;
            Ok(vals.into_iter().map(|v| -v).collect())
        }

        Expr::BinaryOp(lhs, op, rhs) => {
            let left =
                eval_impl(lhs, channels, output_freq, output_len, aliases)?;
            let right =
                eval_impl(rhs, channels, output_freq, output_len, aliases)?;
            let result = left
                .iter()
                .zip(right.iter())
                .map(|(&l, &r)| match op {
                    BinOp::Add => l + r,
                    BinOp::Sub => l - r,
                    BinOp::Mul => l * r,
                    BinOp::Div => {
                        if r == 0.0 {
                            f64::NAN
                        } else {
                            l / r
                        }
                    }
                    BinOp::Mod => {
                        if r == 0.0 {
                            f64::NAN
                        } else {
                            l % r
                        }
                    }
                    BinOp::Gt => if l > r { 1.0 } else { 0.0 },
                    BinOp::Lt => if l < r { 1.0 } else { 0.0 },
                    BinOp::Gte => if l >= r { 1.0 } else { 0.0 },
                    BinOp::Lte => if l <= r { 1.0 } else { 0.0 },
                    BinOp::Eq => if (l - r).abs() < f64::EPSILON { 1.0 } else { 0.0 },
                    BinOp::Neq => if (l - r).abs() >= f64::EPSILON { 1.0 } else { 0.0 },
                    BinOp::And => if l != 0.0 && r != 0.0 { 1.0 } else { 0.0 },
                    BinOp::Or => if l != 0.0 || r != 0.0 { 1.0 } else { 0.0 },
                })
                .collect();
            Ok(result)
        }

        Expr::FuncCall(name, args) => {
            eval_function(name, args, channels, output_freq, output_len, aliases)
        }
    }
}

fn eval_function(
    name: &str,
    args: &[Expr],
    channels: &HashMap<String, ChannelData>,
    freq: u16,
    len: usize,
    aliases: &HashMap<String, String>,
) -> Result<Vec<f64>, MathError> {
    match name {
        // smooth(channel, window_size)
        "smooth" => {
            if args.len() != 2 {
                return Err(MathError {
                    message: "smooth() requires 2 arguments: smooth(channel, window_size)".into(),
                });
            }
            let data = eval_impl(&args[0], channels, freq, len, aliases)?;
            let window = match &args[1] {
                Expr::Number(n) => *n as usize,
                _ => {
                    let w = eval_impl(&args[1], channels, freq, len, aliases)?;
                    w[0] as usize
                }
            };
            Ok(moving_average(&data, window.max(1)))
        }

        // derivative(channel) — finite difference * freq
        "derivative" => {
            if args.len() != 1 {
                return Err(MathError {
                    message: "derivative() requires 1 argument".into(),
                });
            }
            let data = eval_impl(&args[0], channels, freq, len, aliases)?;
            Ok(finite_derivative(&data, freq))
        }

        // integrate(channel) — cumulative sum / freq
        "integrate" => {
            if args.len() != 1 {
                return Err(MathError {
                    message: "integrate() requires 1 argument".into(),
                });
            }
            let data = eval_impl(&args[0], channels, freq, len, aliases)?;
            Ok(cumulative_integral(&data, freq))
        }

        // Single-argument math functions
        "abs" => unary_fn(args, channels, freq, len, aliases, f64::abs),
        "sqrt" => unary_fn(args, channels, freq, len, aliases, f64::sqrt),
        "sin" => unary_fn(args, channels, freq, len, aliases, f64::sin),
        "cos" => unary_fn(args, channels, freq, len, aliases, f64::cos),
        "tan" => unary_fn(args, channels, freq, len, aliases, f64::tan),
        "asin" => unary_fn(args, channels, freq, len, aliases, f64::asin),
        "acos" => unary_fn(args, channels, freq, len, aliases, f64::acos),
        "atan" => unary_fn(args, channels, freq, len, aliases, f64::atan),
        "log" | "ln" => unary_fn(args, channels, freq, len, aliases, f64::ln),
        "exp" => unary_fn(args, channels, freq, len, aliases, f64::exp),
        "floor" => unary_fn(args, channels, freq, len, aliases, f64::floor),
        "ceil" => unary_fn(args, channels, freq, len, aliases, f64::ceil),
        "round" => unary_fn(args, channels, freq, len, aliases, f64::round),

        // Two-argument functions
        "atan2" => binary_fn(args, channels, freq, len, aliases, f64::atan2),
        "pow" => binary_fn(args, channels, freq, len, aliases, f64::powf),
        "min" => binary_fn(args, channels, freq, len, aliases, f64::min),
        "max" => binary_fn(args, channels, freq, len, aliases, f64::max),

        // clamp(value, min, max)
        "clamp" => {
            if args.len() != 3 {
                return Err(MathError {
                    message: "clamp() requires 3 arguments: clamp(value, min, max)".into(),
                });
            }
            let val = eval_impl(&args[0], channels, freq, len, aliases)?;
            let lo = eval_impl(&args[1], channels, freq, len, aliases)?;
            let hi = eval_impl(&args[2], channels, freq, len, aliases)?;
            Ok(val
                .iter()
                .zip(lo.iter())
                .zip(hi.iter())
                .map(|((&v, &l), &h)| v.clamp(l, h))
                .collect())
        }

        // gate(data, condition) — returns data where condition is non-zero, NAN otherwise
        "gate" => {
            if args.len() != 2 {
                return Err(MathError {
                    message: "gate() requires 2 arguments: gate(data, condition)".into(),
                });
            }
            let data = eval_impl(&args[0], channels, freq, len, aliases)?;
            let cond = eval_impl(&args[1], channels, freq, len, aliases)?;
            Ok(data
                .iter()
                .zip(cond.iter())
                .map(|(&d, &c)| if c != 0.0 { d } else { f64::NAN })
                .collect())
        }

        // if_then(condition, true_value, false_value)
        "if_then" => {
            if args.len() != 3 {
                return Err(MathError {
                    message: "if_then() requires 3 arguments: if_then(condition, true_val, false_val)".into(),
                });
            }
            let cond = eval_impl(&args[0], channels, freq, len, aliases)?;
            let true_val = eval_impl(&args[1], channels, freq, len, aliases)?;
            let false_val = eval_impl(&args[2], channels, freq, len, aliases)?;
            Ok(cond
                .iter()
                .zip(true_val.iter())
                .zip(false_val.iter())
                .map(|((&c, &t), &f)| if c != 0.0 { t } else { f })
                .collect())
        }

        // Unit conversion functions
        "kmh_to_mph" => unary_fn(args, channels, freq, len, aliases, |v| v * 0.621371),
        "mph_to_kmh" => unary_fn(args, channels, freq, len, aliases, |v| v * 1.60934),
        "c_to_f" => unary_fn(args, channels, freq, len, aliases, |v| v * 9.0 / 5.0 + 32.0),
        "f_to_c" => unary_fn(args, channels, freq, len, aliases, |v| (v - 32.0) * 5.0 / 9.0),
        "kpa_to_psi" => unary_fn(args, channels, freq, len, aliases, |v| v * 0.145038),
        "psi_to_kpa" => unary_fn(args, channels, freq, len, aliases, |v| v * 6.89476),
        "bar_to_psi" => unary_fn(args, channels, freq, len, aliases, |v| v * 14.5038),
        "psi_to_bar" => unary_fn(args, channels, freq, len, aliases, |v| v / 14.5038),
        "deg_to_rad" => unary_fn(args, channels, freq, len, aliases, |v| v.to_radians()),
        "rad_to_deg" => unary_fn(args, channels, freq, len, aliases, |v| v.to_degrees()),
        "kg_to_lb" => unary_fn(args, channels, freq, len, aliases, |v| v * 2.20462),
        "lb_to_kg" => unary_fn(args, channels, freq, len, aliases, |v| v * 0.453592),
        "m_to_ft" => unary_fn(args, channels, freq, len, aliases, |v| v * 3.28084),
        "ft_to_m" => unary_fn(args, channels, freq, len, aliases, |v| v * 0.3048),
        "nm_to_lbft" => unary_fn(args, channels, freq, len, aliases, |v| v * 0.737562),
        "lbft_to_nm" => unary_fn(args, channels, freq, len, aliases, |v| v * 1.35582),

        _ => Err(MathError {
            message: format!("unknown function '{}'", name),
        }),
    }
}

fn unary_fn(
    args: &[Expr],
    channels: &HashMap<String, ChannelData>,
    freq: u16,
    len: usize,
    aliases: &HashMap<String, String>,
    f: fn(f64) -> f64,
) -> Result<Vec<f64>, MathError> {
    if args.len() != 1 {
        return Err(MathError {
            message: "function requires 1 argument".into(),
        });
    }
    let data = eval_impl(&args[0], channels, freq, len, aliases)?;
    Ok(data.into_iter().map(f).collect())
}

fn binary_fn(
    args: &[Expr],
    channels: &HashMap<String, ChannelData>,
    freq: u16,
    len: usize,
    aliases: &HashMap<String, String>,
    f: fn(f64, f64) -> f64,
) -> Result<Vec<f64>, MathError> {
    if args.len() != 2 {
        return Err(MathError {
            message: "function requires 2 arguments".into(),
        });
    }
    let a = eval_impl(&args[0], channels, freq, len, aliases)?;
    let b = eval_impl(&args[1], channels, freq, len, aliases)?;
    Ok(a.iter().zip(b.iter()).map(|(&x, &y)| f(x, y)).collect())
}

// ---------------------------------------------------------------------------
// DSP helpers
// ---------------------------------------------------------------------------

fn moving_average(data: &[f64], window: usize) -> Vec<f64> {
    if data.is_empty() || window == 0 {
        return data.to_vec();
    }
    let mut result = Vec::with_capacity(data.len());
    let mut sum = 0.0;
    let mut count = 0usize;

    for (i, &val) in data.iter().enumerate() {
        sum += val;
        count += 1;
        if count > window {
            sum -= data[i - window];
            count = window;
        }
        result.push(sum / count as f64);
    }
    result
}

fn finite_derivative(data: &[f64], freq: u16) -> Vec<f64> {
    if data.len() < 2 {
        return vec![0.0; data.len()];
    }
    let f = freq as f64;
    let mut result = Vec::with_capacity(data.len());
    result.push((data[1] - data[0]) * f);
    for i in 1..data.len() - 1 {
        result.push((data[i + 1] - data[i - 1]) * f / 2.0);
    }
    result.push((data[data.len() - 1] - data[data.len() - 2]) * f);
    result
}

fn cumulative_integral(data: &[f64], freq: u16) -> Vec<f64> {
    if data.is_empty() {
        return Vec::new();
    }
    let dt = 1.0 / freq as f64;
    let mut result = Vec::with_capacity(data.len());
    let mut sum = 0.0;
    result.push(0.0);
    for i in 0..data.len() - 1 {
        sum += (data[i] + data[i + 1]) * 0.5 * dt;
        result.push(sum);
    }
    result
}

// ---------------------------------------------------------------------------
// High-level API
// ---------------------------------------------------------------------------

/// Determine the output frequency for an expression: max freq of all referenced channels.
pub fn determine_output_freq(
    expr: &Expr,
    channels: &HashMap<String, ChannelData>,
) -> u16 {
    output_freq_impl(expr, channels, &EMPTY_ALIASES)
}

fn output_freq_impl(
    expr: &Expr,
    channels: &HashMap<String, ChannelData>,
    aliases: &HashMap<String, String>,
) -> u16 {
    let refs = referenced_channels(expr);
    let mut max_freq = 1u16;
    for name in &refs {
        if let Some(resolved) = resolve_channel_name(name, channels, aliases) {
            let f = channels[resolved].freq;
            if f > max_freq {
                max_freq = f;
            }
        }
    }
    max_freq
}

/// Determine the output length for an expression at a given frequency.
pub fn determine_output_len(
    expr: &Expr,
    channels: &HashMap<String, ChannelData>,
    output_freq: u16,
) -> usize {
    output_len_impl(expr, channels, output_freq, &EMPTY_ALIASES)
}

fn output_len_impl(
    expr: &Expr,
    channels: &HashMap<String, ChannelData>,
    output_freq: u16,
    aliases: &HashMap<String, String>,
) -> usize {
    let refs = referenced_channels(expr);
    let mut max_duration: f64 = 0.0;
    for name in &refs {
        if let Some(resolved) = resolve_channel_name(name, channels, aliases) {
            let ch = &channels[resolved];
            if ch.freq > 0 {
                let dur = ch.samples.len() as f64 / ch.freq as f64;
                if dur > max_duration {
                    max_duration = dur;
                }
            }
        }
    }
    (max_duration * output_freq as f64).ceil() as usize
}

/// Parse and evaluate a math expression string.
pub fn evaluate_expression(
    expression: &str,
    channels: &HashMap<String, ChannelData>,
) -> Result<(Vec<f64>, u16), String> {
    evaluate_expression_with_aliases(expression, channels, &EMPTY_ALIASES)
}

/// Parse and evaluate a math expression string, with channel alias support.
pub fn evaluate_expression_with_aliases(
    expression: &str,
    channels: &HashMap<String, ChannelData>,
    aliases: &HashMap<String, String>,
) -> Result<(Vec<f64>, u16), String> {
    let expr = parse_expression(expression).map_err(|e| e.to_string())?;
    let freq = output_freq_impl(&expr, channels, aliases);
    let len = output_len_impl(&expr, channels, freq, aliases);
    if len == 0 {
        return Err("expression references no channels with data".into());
    }
    let samples = eval_impl(&expr, channels, freq, len, aliases)
        .map_err(|e| e.to_string())?;
    Ok((samples, freq))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_channels() -> HashMap<String, ChannelData> {
        let mut m = HashMap::new();
        m.insert(
            "Speed".into(),
            ChannelData {
                samples: vec![10.0, 20.0, 30.0, 40.0, 50.0],
                freq: 1,
            },
        );
        m.insert(
            "RPM".into(),
            ChannelData {
                samples: vec![1000.0, 2000.0, 3000.0, 4000.0, 5000.0],
                freq: 1,
            },
        );
        m.insert(
            "Engine Speed".into(),
            ChannelData {
                samples: vec![100.0, 200.0, 300.0, 400.0, 500.0],
                freq: 1,
            },
        );
        m
    }

    #[test]
    fn eval_constant() {
        let channels = make_channels();
        // Pure constants with no channel references produce an error (no context for length).
        let result = evaluate_expression("42", &channels);
        assert!(result.is_err());
    }

    #[test]
    fn eval_simple_arithmetic() {
        let channels = make_channels();
        let (result, _) = evaluate_expression("Speed + 5", &channels).unwrap();
        assert_eq!(result, vec![15.0, 25.0, 35.0, 45.0, 55.0]);
    }

    #[test]
    fn eval_channel_arithmetic() {
        let channels = make_channels();
        let (result, _) = evaluate_expression("RPM / Speed", &channels).unwrap();
        assert_eq!(result, vec![100.0, 100.0, 100.0, 100.0, 100.0]);
    }

    #[test]
    fn eval_quoted_channel() {
        let channels = make_channels();
        let (result, _) = evaluate_expression("\"Engine Speed\" / 100", &channels).unwrap();
        assert_eq!(result, vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    }

    #[test]
    fn eval_underscore_resolution() {
        let channels = make_channels();
        let (result, _) = evaluate_expression("Engine_Speed / 100", &channels).unwrap();
        assert_eq!(result, vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    }

    #[test]
    fn eval_unknown_channel() {
        let channels = make_channels();
        let result = evaluate_expression("NonExistent + 1", &channels);
        assert!(result.is_err());
    }

    #[test]
    fn eval_derivative() {
        let channels = make_channels();
        // Speed = [10, 20, 30, 40, 50] at 1Hz
        // derivative should be ~10 everywhere
        let (result, _) = evaluate_expression("derivative(Speed)", &channels).unwrap();
        assert_eq!(result.len(), 5);
        // Central differences: [10, 10, 10, 10, 10]
        for &v in &result {
            assert!((v - 10.0).abs() < 1e-10);
        }
    }

    #[test]
    fn eval_smooth() {
        let channels = make_channels();
        let (result, _) = evaluate_expression("smooth(Speed, 3)", &channels).unwrap();
        assert_eq!(result.len(), 5);
        // Moving average with window 3:
        // [10/1, (10+20)/2, (10+20+30)/3, (20+30+40)/3, (30+40+50)/3]
        assert!((result[0] - 10.0).abs() < 1e-10);
        assert!((result[1] - 15.0).abs() < 1e-10);
        assert!((result[2] - 20.0).abs() < 1e-10);
        assert!((result[3] - 30.0).abs() < 1e-10);
        assert!((result[4] - 40.0).abs() < 1e-10);
    }

    #[test]
    fn eval_integrate() {
        let channels = make_channels();
        // Speed = [10, 20, 30, 40, 50] at 1Hz, dt = 1.0
        // Trapezoidal rule: [0, 15, 40, 75, 120]
        let (result, _) = evaluate_expression("integrate(Speed)", &channels).unwrap();
        assert_eq!(result, vec![0.0, 15.0, 40.0, 75.0, 120.0]);
    }

    #[test]
    fn eval_abs_neg() {
        let channels = make_channels();
        let (result, _) = evaluate_expression("abs(-Speed)", &channels).unwrap();
        assert_eq!(result, vec![10.0, 20.0, 30.0, 40.0, 50.0]);
    }

    #[test]
    fn eval_nested_functions() {
        let channels = make_channels();
        let (result, _) = evaluate_expression("abs(derivative(Speed))", &channels).unwrap();
        assert_eq!(result.len(), 5);
        for &v in &result {
            assert!((v - 10.0).abs() < 1e-10);
        }
    }

    #[test]
    fn eval_division_by_zero() {
        let channels = make_channels();
        let (result, _) = evaluate_expression("Speed / (Speed - Speed)", &channels).unwrap();
        for &v in &result {
            assert!(v.is_nan());
        }
    }

    #[test]
    fn eval_complex_expression() {
        let channels = make_channels();
        // (RPM - Speed * 100) / (Speed * 100) * 100
        // = (1000 - 1000) / 1000 * 100 = 0 for first sample, etc.
        let (result, _) =
            evaluate_expression("(RPM - Speed * 100) / (Speed * 100) * 100", &channels).unwrap();
        for &v in &result {
            assert!(v.abs() < 1e-10);
        }
    }

    #[test]
    fn eval_resample_different_freqs() {
        let mut channels = HashMap::new();
        channels.insert(
            "Fast".into(),
            ChannelData {
                samples: vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0],
                freq: 10,
            },
        );
        channels.insert(
            "Slow".into(),
            ChannelData {
                samples: vec![0.0, 10.0],
                freq: 2,
            },
        );
        // Output freq should be 10 (max). Slow gets resampled from 2Hz to 10Hz.
        let (result, freq) = evaluate_expression("Fast + Slow", &channels).unwrap();
        assert_eq!(freq, 10);
        assert_eq!(result.len(), 10);
        // First sample: 0 + 0 = 0
        assert!((result[0] - 0.0).abs() < 1e-10);
    }

    #[test]
    fn eval_comparison_operators() {
        let channels = make_channels();
        // Speed = [10, 20, 30, 40, 50]
        let (result, _) = evaluate_expression("Speed > 25", &channels).unwrap();
        assert_eq!(result, vec![0.0, 0.0, 1.0, 1.0, 1.0]);

        let (result, _) = evaluate_expression("Speed <= 30", &channels).unwrap();
        assert_eq!(result, vec![1.0, 1.0, 1.0, 0.0, 0.0]);

        let (result, _) = evaluate_expression("Speed == 30", &channels).unwrap();
        assert_eq!(result, vec![0.0, 0.0, 1.0, 0.0, 0.0]);

        let (result, _) = evaluate_expression("Speed != 30", &channels).unwrap();
        assert_eq!(result, vec![1.0, 1.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn eval_logical_operators() {
        let channels = make_channels();
        // Speed > 15 && Speed < 45
        let (result, _) = evaluate_expression("Speed > 15 && Speed < 45", &channels).unwrap();
        assert_eq!(result, vec![0.0, 1.0, 1.0, 1.0, 0.0]);

        // Speed < 15 || Speed > 45
        let (result, _) = evaluate_expression("Speed < 15 || Speed > 45", &channels).unwrap();
        assert_eq!(result, vec![1.0, 0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn eval_gate() {
        let channels = make_channels();
        // gate(Speed, Speed > 25) — keep values where Speed > 25, NAN otherwise
        let (result, _) = evaluate_expression("gate(Speed, Speed > 25)", &channels).unwrap();
        assert!(result[0].is_nan());
        assert!(result[1].is_nan());
        assert_eq!(result[2], 30.0);
        assert_eq!(result[3], 40.0);
        assert_eq!(result[4], 50.0);
    }

    #[test]
    fn eval_if_then() {
        let channels = make_channels();
        // if_then(Speed > 25, Speed, 0) — Speed where > 25, else 0
        let (result, _) = evaluate_expression("if_then(Speed > 25, Speed, 0)", &channels).unwrap();
        assert_eq!(result, vec![0.0, 0.0, 30.0, 40.0, 50.0]);
    }

    #[test]
    fn eval_unit_conversion() {
        let channels = make_channels();
        // Speed = [10, 20, 30, 40, 50] in km/h
        let (result, _) = evaluate_expression("kmh_to_mph(Speed)", &channels).unwrap();
        for (i, &v) in result.iter().enumerate() {
            let expected = (i as f64 + 1.0) * 10.0 * 0.621371;
            assert!((v - expected).abs() < 1e-6);
        }

        // Round-trip: mph_to_kmh(kmh_to_mph(Speed)) ≈ Speed
        let (result, _) = evaluate_expression("mph_to_kmh(kmh_to_mph(Speed))", &channels).unwrap();
        for (i, &v) in result.iter().enumerate() {
            let expected = (i as f64 + 1.0) * 10.0;
            assert!((v - expected).abs() < 1e-3, "round-trip mismatch: {} vs {}", v, expected);
        }
    }

    #[test]
    fn eval_with_aliases() {
        let channels = make_channels();
        // "Velocity" is not a real channel, but alias it to "Speed"
        let mut aliases = HashMap::new();
        aliases.insert("Velocity".into(), "Speed".into());

        let (result, _) =
            evaluate_expression_with_aliases("Velocity + 5", &channels, &aliases).unwrap();
        assert_eq!(result, vec![15.0, 25.0, 35.0, 45.0, 55.0]);

        // Alias with different casing
        aliases.insert("Revs".into(), "RPM".into());
        let (result, _) =
            evaluate_expression_with_aliases("Revs / Velocity", &channels, &aliases).unwrap();
        assert_eq!(result, vec![100.0, 100.0, 100.0, 100.0, 100.0]);
    }
}
