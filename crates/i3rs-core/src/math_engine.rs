//! Math channel evaluator: evaluates parsed expressions against channel data.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, LazyLock, RwLock};

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

fn normalize_channel_name(name: &str) -> String {
    name.to_ascii_lowercase().replace(['.', '_'], " ")
}

fn resolve_actual_channel_name<'a>(
    reference: &str,
    available: &'a HashMap<String, ChannelData>,
    normalized_name_index: &HashMap<String, String>,
) -> Option<&'a str> {
    if let Some((key, _)) = available.get_key_value(reference) {
        return Some(key);
    }

    let with_spaces = reference.replace('_', " ");
    if let Some((key, _)) = available.get_key_value(&with_spaces) {
        return Some(key);
    }

    let with_dots = reference.replace('_', ".");
    if let Some((key, _)) = available.get_key_value(&with_dots) {
        return Some(key);
    }

    let normalized = normalize_channel_name(reference);
    normalized_name_index
        .get(&normalized)
        .and_then(|key| available.get_key_value(key).map(|(key, _)| key.as_str()))
}

#[allow(dead_code)]
/// Resolve a channel reference against available channel names.
///
/// Resolution priority: exact → underscore-to-space → underscore-to-dot
/// → alias (exact + normalized) → case-insensitive channel → case-insensitive alias.
fn resolve_channel_name<'a>(
    reference: &str,
    available: &'a HashMap<String, ChannelData>,
    aliases: &HashMap<String, String>,
) -> Option<&'a str> {
    let mut normalized_name_index = HashMap::new();
    for key in available.keys() {
        normalized_name_index
            .entry(normalize_channel_name(key))
            .or_insert_with(|| key.clone());
    }

    if let Some(key) = resolve_actual_channel_name(reference, available, &normalized_name_index) {
        return Some(key);
    }

    let with_spaces = reference.replace('_', " ");
    let with_dots = reference.replace('_', ".");
    for variant in [reference, with_spaces.as_str(), with_dots.as_str()] {
        if let Some(target) = aliases.get(variant)
            && let Some(key) =
                resolve_actual_channel_name(target, available, &normalized_name_index)
        {
            return Some(key);
        }
    }

    let normalized_reference = normalize_channel_name(reference);
    for (alias, target) in aliases {
        if normalize_channel_name(alias) == normalized_reference
            && let Some(key) =
                resolve_actual_channel_name(target, available, &normalized_name_index)
        {
            return Some(key);
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
    for (alias, target) in aliases {
        if alias.eq_ignore_ascii_case(reference) {
            return Some(target.clone());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Resampling
// ---------------------------------------------------------------------------

/// Resample a channel to a target frequency using linear interpolation.
fn resample(data: &[f64], src_freq: u16, target_freq: u16, target_len: usize) -> Vec<f64> {
    if src_freq == target_freq && data.len() == target_len {
        return data.to_vec();
    }
    if data.is_empty() {
        return vec![0.0; target_len];
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
    out
}

// ---------------------------------------------------------------------------
// Evaluator
// ---------------------------------------------------------------------------

static EMPTY_ALIASES: LazyLock<HashMap<String, String>> = LazyLock::new(HashMap::new);
static PARSED_EXPRESSION_CACHE: LazyLock<RwLock<HashMap<String, Arc<Expr>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

fn parse_expression_cached(expression: &str) -> Result<Arc<Expr>, crate::math_expr::ParseError> {
    if let Ok(cache) = PARSED_EXPRESSION_CACHE.read()
        && let Some(parsed) = cache.get(expression)
    {
        return Ok(Arc::clone(parsed));
    }

    let parsed = Arc::new(parse_expression(expression)?);
    if let Ok(mut cache) = PARSED_EXPRESSION_CACHE.write() {
        cache.insert(expression.to_string(), Arc::clone(&parsed));
    }
    Ok(parsed)
}

struct ChannelResolver<'a> {
    channels: &'a HashMap<String, ChannelData>,
    normalized_name_index: HashMap<String, String>,
    alias_name_index: HashMap<String, String>,
}

impl<'a> ChannelResolver<'a> {
    fn new(channels: &'a HashMap<String, ChannelData>, aliases: &HashMap<String, String>) -> Self {
        let mut normalized_name_index = HashMap::new();
        for key in channels.keys() {
            normalized_name_index
                .entry(normalize_channel_name(key))
                .or_insert_with(|| key.clone());
        }

        let mut alias_name_index = HashMap::new();
        for (alias, target) in aliases {
            if let Some(actual) =
                resolve_actual_channel_name(target, channels, &normalized_name_index)
            {
                alias_name_index.insert(alias.clone(), actual.to_string());
                alias_name_index
                    .entry(normalize_channel_name(alias))
                    .or_insert_with(|| actual.to_string());
            }
        }

        Self {
            channels,
            normalized_name_index,
            alias_name_index,
        }
    }

    fn resolve(&self, reference: &str) -> Option<&str> {
        if let Some(key) =
            resolve_actual_channel_name(reference, self.channels, &self.normalized_name_index)
        {
            return Some(key);
        }

        let with_spaces = reference.replace('_', " ");
        let with_dots = reference.replace('_', ".");
        for variant in [reference, with_spaces.as_str(), with_dots.as_str()] {
            if let Some(actual) = self.alias_name_index.get(variant) {
                return Some(actual.as_str());
            }
        }

        let normalized = normalize_channel_name(reference);
        self.alias_name_index
            .get(&normalized)
            .or_else(|| self.normalized_name_index.get(&normalized))
            .map(String::as_str)
    }

    fn get_channel(&self, reference: &str) -> Option<(&str, &'a ChannelData)> {
        let key = self.resolve(reference)?;
        self.channels
            .get_key_value(key)
            .map(|(key, value)| (key.as_str(), value))
    }
}

struct EvaluationContext<'a> {
    resolver: ChannelResolver<'a>,
    output_freq: u16,
    output_len: usize,
    resample_cache: HashMap<(String, u16, u16, usize), Arc<Vec<f64>>>,
    node_outputs: HashMap<usize, Arc<Vec<f64>>>,
}

impl<'a> EvaluationContext<'a> {
    fn new(
        channels: &'a HashMap<String, ChannelData>,
        aliases: &'a HashMap<String, String>,
        output_freq: u16,
        output_len: usize,
    ) -> Self {
        Self {
            resolver: ChannelResolver::new(channels, aliases),
            output_freq,
            output_len,
            resample_cache: HashMap::new(),
            node_outputs: HashMap::new(),
        }
    }

    fn evaluate(&mut self, expr: &Expr) -> Result<Arc<Vec<f64>>, MathError> {
        let node_id = expr as *const Expr as usize;
        if let Some(cached) = self.node_outputs.get(&node_id) {
            return Ok(Arc::clone(cached));
        }

        let output = match expr {
            Expr::Number(n) => Arc::new(vec![*n; self.output_len]),

            Expr::Channel(name) => {
                let (resolved, channel) =
                    self.resolver.get_channel(name).ok_or_else(|| MathError {
                        message: format!("unknown channel '{}'", name),
                    })?;
                let cache_key = (
                    resolved.to_string(),
                    channel.freq,
                    self.output_freq,
                    self.output_len,
                );
                if let Some(cached) = self.resample_cache.get(&cache_key) {
                    Arc::clone(cached)
                } else {
                    let resampled = if channel.freq == self.output_freq
                        && channel.samples.len() == self.output_len
                    {
                        channel.samples.clone()
                    } else {
                        resample(
                            &channel.samples,
                            channel.freq,
                            self.output_freq,
                            self.output_len,
                        )
                    };
                    let resampled = Arc::new(resampled);
                    self.resample_cache
                        .insert(cache_key, Arc::clone(&resampled));
                    resampled
                }
            }

            Expr::UnaryNeg(inner) => {
                let vals = self.evaluate(inner)?;
                Arc::new(vals.iter().map(|v| -*v).collect())
            }

            Expr::BinaryOp(lhs, op, rhs) => {
                let left = self.evaluate(lhs)?;
                let right = self.evaluate(rhs)?;
                Arc::new(
                    left.iter()
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
                            BinOp::Gt => {
                                if l > r {
                                    1.0
                                } else {
                                    0.0
                                }
                            }
                            BinOp::Lt => {
                                if l < r {
                                    1.0
                                } else {
                                    0.0
                                }
                            }
                            BinOp::Gte => {
                                if l >= r {
                                    1.0
                                } else {
                                    0.0
                                }
                            }
                            BinOp::Lte => {
                                if l <= r {
                                    1.0
                                } else {
                                    0.0
                                }
                            }
                            BinOp::Eq => {
                                if l == r {
                                    1.0
                                } else {
                                    0.0
                                }
                            }
                            BinOp::Neq => {
                                if l != r {
                                    1.0
                                } else {
                                    0.0
                                }
                            }
                            BinOp::And => {
                                if !l.is_nan() && l != 0.0 && !r.is_nan() && r != 0.0 {
                                    1.0
                                } else {
                                    0.0
                                }
                            }
                            BinOp::Or => {
                                if (!l.is_nan() && l != 0.0) || (!r.is_nan() && r != 0.0) {
                                    1.0
                                } else {
                                    0.0
                                }
                            }
                        })
                        .collect(),
                )
            }

            Expr::FuncCall(name, args) => self.evaluate_function(name, args)?,
        };

        self.node_outputs.insert(node_id, Arc::clone(&output));
        Ok(output)
    }

    fn evaluate_function(&mut self, name: &str, args: &[Expr]) -> Result<Arc<Vec<f64>>, MathError> {
        match name {
            "smooth" => {
                if args.len() != 2 {
                    return Err(MathError {
                        message: "smooth() requires 2 arguments: smooth(channel, window_size)"
                            .into(),
                    });
                }
                let data = self.evaluate(&args[0])?;
                let window = match &args[1] {
                    Expr::Number(n) => *n as usize,
                    _ => self.evaluate(&args[1])?[0] as usize,
                };
                Ok(Arc::new(moving_average(&data, window.max(1))))
            }
            "derivative" => {
                if args.len() != 1 {
                    return Err(MathError {
                        message: "derivative() requires 1 argument".into(),
                    });
                }
                let data = self.evaluate(&args[0])?;
                Ok(Arc::new(finite_derivative(&data, self.output_freq)))
            }
            "integrate" => {
                if args.len() != 1 {
                    return Err(MathError {
                        message: "integrate() requires 1 argument".into(),
                    });
                }
                let data = self.evaluate(&args[0])?;
                Ok(Arc::new(cumulative_integral(&data, self.output_freq)))
            }
            "abs" => self.unary_fn(args, f64::abs),
            "sqrt" => self.unary_fn(args, f64::sqrt),
            "sin" => self.unary_fn(args, f64::sin),
            "cos" => self.unary_fn(args, f64::cos),
            "tan" => self.unary_fn(args, f64::tan),
            "asin" => self.unary_fn(args, f64::asin),
            "acos" => self.unary_fn(args, f64::acos),
            "atan" => self.unary_fn(args, f64::atan),
            "log" | "ln" => self.unary_fn(args, f64::ln),
            "exp" => self.unary_fn(args, f64::exp),
            "floor" => self.unary_fn(args, f64::floor),
            "ceil" => self.unary_fn(args, f64::ceil),
            "round" => self.unary_fn(args, f64::round),
            "atan2" => self.binary_fn(args, f64::atan2),
            "pow" => self.binary_fn(args, f64::powf),
            "min" => self.binary_fn(args, f64::min),
            "max" => self.binary_fn(args, f64::max),
            "clamp" => {
                if args.len() != 3 {
                    return Err(MathError {
                        message: "clamp() requires 3 arguments: clamp(value, min, max)".into(),
                    });
                }
                let val = self.evaluate(&args[0])?;
                let lo = self.evaluate(&args[1])?;
                let hi = self.evaluate(&args[2])?;
                Ok(Arc::new(
                    val.iter()
                        .zip(lo.iter())
                        .zip(hi.iter())
                        .map(|((&v, &l), &h)| v.clamp(l, h))
                        .collect(),
                ))
            }
            "gate" => {
                if args.len() != 2 {
                    return Err(MathError {
                        message: "gate() requires 2 arguments: gate(data, condition)".into(),
                    });
                }
                let data = self.evaluate(&args[0])?;
                let cond = self.evaluate(&args[1])?;
                Ok(Arc::new(
                    data.iter()
                        .zip(cond.iter())
                        .map(|(&d, &c)| if c != 0.0 { d } else { f64::NAN })
                        .collect(),
                ))
            }
            "if_then" => {
                if args.len() != 3 {
                    return Err(MathError {
                        message:
                            "if_then() requires 3 arguments: if_then(condition, true_val, false_val)"
                                .into(),
                    });
                }
                let cond = self.evaluate(&args[0])?;
                let true_val = self.evaluate(&args[1])?;
                let false_val = self.evaluate(&args[2])?;
                Ok(Arc::new(
                    cond.iter()
                        .zip(true_val.iter())
                        .zip(false_val.iter())
                        .map(|((&c, &t), &f)| if c != 0.0 { t } else { f })
                        .collect(),
                ))
            }
            "kmh_to_mph" => self.unary_fn(args, |v| v * 0.621371),
            "mph_to_kmh" => self.unary_fn(args, |v| v * 1.60934),
            "c_to_f" => self.unary_fn(args, |v| v * 9.0 / 5.0 + 32.0),
            "f_to_c" => self.unary_fn(args, |v| (v - 32.0) * 5.0 / 9.0),
            "kpa_to_psi" => self.unary_fn(args, |v| v * 0.145038),
            "psi_to_kpa" => self.unary_fn(args, |v| v * 6.89476),
            "bar_to_psi" => self.unary_fn(args, |v| v * 14.5038),
            "psi_to_bar" => self.unary_fn(args, |v| v / 14.5038),
            "deg_to_rad" => self.unary_fn(args, f64::to_radians),
            "rad_to_deg" => self.unary_fn(args, f64::to_degrees),
            "kg_to_lb" => self.unary_fn(args, |v| v * 2.20462),
            "lb_to_kg" => self.unary_fn(args, |v| v * 0.453592),
            "m_to_ft" => self.unary_fn(args, |v| v * 3.28084),
            "ft_to_m" => self.unary_fn(args, |v| v * 0.3048),
            "nm_to_lbft" => self.unary_fn(args, |v| v * 0.737562),
            "lbft_to_nm" => self.unary_fn(args, |v| v * 1.35582),
            _ => Err(MathError {
                message: format!("unknown function '{}'", name),
            }),
        }
    }

    fn unary_fn(&mut self, args: &[Expr], f: fn(f64) -> f64) -> Result<Arc<Vec<f64>>, MathError> {
        if args.len() != 1 {
            return Err(MathError {
                message: "function requires 1 argument".into(),
            });
        }
        let data = self.evaluate(&args[0])?;
        Ok(Arc::new(data.iter().copied().map(f).collect()))
    }

    fn binary_fn(
        &mut self,
        args: &[Expr],
        f: fn(f64, f64) -> f64,
    ) -> Result<Arc<Vec<f64>>, MathError> {
        if args.len() != 2 {
            return Err(MathError {
                message: "function requires 2 arguments".into(),
            });
        }
        let a = self.evaluate(&args[0])?;
        let b = self.evaluate(&args[1])?;
        Ok(Arc::new(
            a.iter().zip(b.iter()).map(|(&x, &y)| f(x, y)).collect(),
        ))
    }
}

/// Evaluate a parsed expression against channel data (no aliases).
pub fn evaluate(
    expr: &Expr,
    channels: &HashMap<String, ChannelData>,
    output_freq: u16,
    output_len: usize,
) -> Result<Vec<f64>, MathError> {
    let mut context = EvaluationContext::new(channels, &EMPTY_ALIASES, output_freq, output_len);
    Ok((*context.evaluate(expr)?).clone())
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
pub fn determine_output_freq(expr: &Expr, channels: &HashMap<String, ChannelData>) -> u16 {
    let resolver = ChannelResolver::new(channels, &EMPTY_ALIASES);
    output_freq_impl(expr, &resolver)
}

fn output_freq_impl(expr: &Expr, resolver: &ChannelResolver<'_>) -> u16 {
    let refs = referenced_channels(expr);
    let mut max_freq = 1u16;
    for name in &refs {
        if let Some((_, channel)) = resolver.get_channel(name) {
            let f = channel.freq;
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
    let resolver = ChannelResolver::new(channels, &EMPTY_ALIASES);
    output_len_impl(expr, &resolver, output_freq)
}

fn output_len_impl(expr: &Expr, resolver: &ChannelResolver<'_>, output_freq: u16) -> usize {
    let refs = referenced_channels(expr);
    let mut max_duration: f64 = 0.0;
    for name in &refs {
        if let Some((_, channel)) = resolver.get_channel(name)
            && channel.freq > 0
        {
            let dur = channel.samples.len() as f64 / channel.freq as f64;
            if dur > max_duration {
                max_duration = dur;
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
    let expr = parse_expression_cached(expression).map_err(|e| e.to_string())?;
    let resolver = ChannelResolver::new(channels, aliases);
    let freq = output_freq_impl(&expr, &resolver);
    let len = output_len_impl(&expr, &resolver, freq);
    if len == 0 {
        return Err("expression references no channels with data".into());
    }
    let mut context = EvaluationContext::new(channels, aliases, freq, len);
    let samples = context.evaluate(&expr).map_err(|e| e.to_string())?;
    Ok(((*samples).clone(), freq))
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
            assert!(
                (v - expected).abs() < 1e-3,
                "round-trip mismatch: {} vs {}",
                v,
                expected
            );
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

    #[test]
    fn eval_alias_and_channel_resolution_uses_normalized_names() {
        let channels = HashMap::from([(
            "Vehicle Speed".into(),
            ChannelData {
                samples: vec![1.0, 2.0, 3.0, 4.0],
                freq: 2,
            },
        )]);
        let aliases = HashMap::from([("Speed.Value".into(), "Vehicle_Speed".into())]);

        let (result, _) =
            evaluate_expression_with_aliases("Speed_Value + Vehicle_Speed", &channels, &aliases)
                .unwrap();
        assert_eq!(result, vec![2.0, 4.0, 6.0, 8.0]);
    }
}
