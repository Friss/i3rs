//! Side-elevation schematic primitives for wireframe chassis rendering.
//!
//! [`compute_schematic`] converts a [`ChassisModel`] and [`FrameState`] (or raw pot readings)
//! into lists of [`SchematicLine`] and [`SchematicCircle`] ready for egui `Painter` calls.
//! The caller transforms from solver space (origin = swingarm pivot, +X forward, +Y up,
//! display flips X) to screen coordinates.

use std::f64::consts::PI;

use super::{ChassisModel, FrameState};

// ---------------------------------------------------------------------------
// Public output types
// ---------------------------------------------------------------------------

/// Stroke style for a schematic line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchematicStroke {
    Solid,
    Dashed,
    Dotted,
}

/// A single line primitive in solver space (mm).
#[derive(Debug, Clone, Copy)]
pub struct SchematicLine {
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
    pub stroke: SchematicStroke,
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub thickness: f64,
}

/// A circle primitive in solver space (mm).
#[derive(Debug, Clone, Copy)]
pub struct SchematicCircle {
    pub cx: f64,
    pub cy: f64,
    pub radius_mm: f64,
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub fill: bool,
    pub thickness: f64,
}

/// Axis-aligned bounding box of all primitives.
#[derive(Debug, Clone, Copy, Default)]
pub struct SchematicBounds {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

impl SchematicBounds {
    pub fn width(&self) -> f64 { (self.max_x - self.min_x).max(1e-6) }
    pub fn height(&self) -> f64 { (self.max_y - self.min_y).max(1e-6) }
}

/// A legend entry describing one component colour and style.
#[derive(Debug, Clone)]
pub struct LegendEntry {
    pub label: String,
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub stroke: SchematicStroke,
    pub thickness: f64,
}

/// Full schematic output from [`compute_schematic`].
pub struct SideViewSchematic {
    pub lines: Vec<SchematicLine>,
    pub circles: Vec<SchematicCircle>,
    pub legend: Vec<LegendEntry>,
    pub bounds: SchematicBounds,
}

// ---------------------------------------------------------------------------
// Palette
// ---------------------------------------------------------------------------

const COL_FRAME: (u8, u8, u8) = (60, 110, 180);
const COL_SWING: (u8, u8, u8) = (230, 130, 40);
const COL_FORK: (u8, u8, u8) = (30, 60, 130);
const COL_SHOCK: (u8, u8, u8) = (160, 80, 200);
const COL_CHAIN: (u8, u8, u8) = (110, 80, 45);
const COL_SPROCKET: (u8, u8, u8) = (95, 75, 55);
const COL_GROUND: (u8, u8, u8) = (140, 140, 140);
const COL_TIRE: (u8, u8, u8) = (30, 30, 35);
const COL_RIM: (u8, u8, u8) = (235, 235, 240);
const COL_HUB: (u8, u8, u8) = (90, 90, 95);
const COL_LINK: (u8, u8, u8) = (200, 120, 60);
const COL_STEERING: (u8, u8, u8) = (190, 60, 60);
const COL_TRAIL: (u8, u8, u8) = (200, 100, 100);

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Compute the side-view schematic.
///
/// When `state` is `Some`, uses the solved kinematic angles from the solver for
/// accurate animation. When `None`, falls back to a heuristic pitch estimate from
/// the raw pot readings.
pub fn compute_schematic(
    chassis: Option<&ChassisModel>,
    state: Option<&FrameState>,
    susp_fr_mm: f64,
    susp_rr_mm: f64,
) -> SideViewSchematic {
    match state {
        Some(st) => compute_from_frame_state(chassis, st),
        None => compute_heuristic(chassis, susp_fr_mm, susp_rr_mm),
    }
}

// ---------------------------------------------------------------------------
// Solved-state path
// ---------------------------------------------------------------------------

fn compute_from_frame_state(chassis: Option<&ChassisModel>, state: &FrameState) -> SideViewSchematic {
    let Some(chassis) = chassis else { return empty() };
    let Some(fr) = chassis.frame.as_ref() else { return empty() };
    let sw = chassis.swingarm.as_ref();
    let lk = chassis.link.as_ref();
    let r_r = if chassis.rear_tire_rad_mm > 50.0 { chassis.rear_tire_rad_mm } else { 320.0 };
    let r_f = if chassis.front_tire_rad_mm > 50.0 { chassis.front_tire_rad_mm } else { 300.0 };

    let gamma_rad = state.gamma_rad;
    let theta_rad = state.theta_rad;
    let cg = gamma_rad.cos() as f32;
    let sg = gamma_rad.sin() as f32;
    let ct = theta_rad.cos() as f32;
    let st = theta_rad.sin() as f32;

    let body_to_world = |xd: f64, yd: f64| -> (f32, f32) {
        ((xd * cg as f64 - yd * sg as f64) as f32, (xd * sg as f64 + yd * cg as f64) as f32)
    };

    let rear_axle_y_shift = (r_r - state.rear_axle_y) as f32;
    let rear_axle = (state.rear_axle_x as f32, r_r as f32);
    let pivot = (state.pivot_x as f32, state.pivot_y as f32 + rear_axle_y_shift);

    let axle_offset = sw.map(|s| s.offset).unwrap_or(0.0) as f32;
    let lx_h = {
        let swl = chassis.sw_l_mm as f32;
        if swl > axle_offset { (swl * swl - axle_offset * axle_offset).sqrt() } else { swl }
    };

    let world = |ms_x: f64, ms_y: f64| -> (f32, f32) {
        let vx = (-ms_x) as f32;
        let vy = ms_y as f32;
        let rx = vx * ct - vy * st;
        let ry = vx * st + vy * ct;
        (pivot.0 + rx, pivot.1 + ry)
    };

    let head_angle_rad = (fr.head_angle_deg * PI / 180.0) as f32;
    let steer_ms = (-head_angle_rad.sin(), -head_angle_rad.cos());
    let steer_dir = (steer_ms.0 * ct - steer_ms.1 * st, steer_ms.0 * st + steer_ms.1 * ct);
    let u_len = (steer_dir.0 * steer_dir.0 + steer_dir.1 * steer_dir.1).sqrt();
    let u = if u_len > 1e-6 { (steer_dir.0 / u_len, steer_dir.1 / u_len) } else { (0.0, -1.0) };
    let n = (u.1, -u.0);
    let o = chassis.yoke_offset_mm as f32;

    let head = world(fr.head_x, fr.head_y);
    let link_mnt = world(fr.link_mnt_x, fr.link_mnt_y);
    let c_shaft = world(fr.c_shaft_x, fr.c_shaft_y);

    let front_axle = if u.1.abs() > 1e-4 {
        let fb = (head.0 + o * n.0, head.1 + o * n.1);
        let s = (r_f as f32 - fb.1) / u.1;
        let candidate = (fb.0 + s * u.0, fb.1 + s * u.1);
        if candidate.0.is_finite() { candidate } else { (0.0, r_f as f32) }
    } else {
        (0.0, r_f as f32)
    };

    let _ = (lx_h, axle_offset, body_to_world, &lk);

    draw_common(
        chassis,
        front_axle, rear_axle, pivot, head, link_mnt, c_shaft,
        u, n, o,
        r_f, r_r,
    )
}

// ---------------------------------------------------------------------------
// Heuristic fallback
// ---------------------------------------------------------------------------

fn compute_heuristic(chassis: Option<&ChassisModel>, susp_fr_mm: f64, susp_rr_mm: f64) -> SideViewSchematic {
    let Some(chassis) = chassis else { return empty() };
    let Some(fr) = chassis.frame.as_ref() else { return empty() };
    let sw = chassis.swingarm.as_ref();
    let r_r = if chassis.rear_tire_rad_mm > 50.0 { chassis.rear_tire_rad_mm } else { 320.0 };
    let r_f = if chassis.front_tire_rad_mm > 50.0 { chassis.front_tire_rad_mm } else { 300.0 };
    let whl = if chassis.wheel_center_hypotenuse_mm > 500.0 { chassis.wheel_center_hypotenuse_mm } else { 1480.0 };
    let sw_l = if chassis.sw_l_mm > 200.0 { chassis.sw_l_mm } else { 620.0 };
    let shock_stroke = chassis.shock.as_ref().filter(|s| s.stroke_mm > 1.0).map(|s| s.stroke_mm).unwrap_or(75.0);

    let wb_horiz = (whl * whl - (r_f - r_r).powi(2)).max(0.0).sqrt();
    let axle_offset = sw.map(|s| s.offset).unwrap_or(0.0).max(0.0).min(sw_l * 0.9);
    let axle_offset = if axle_offset < 0.0 || axle_offset >= sw_l { sw_l * 0.15 } else { axle_offset };
    let lx_h = (sw_l * sw_l - axle_offset * axle_offset).max(1.0).sqrt();

    let gamma_design_rad = if let Some(drop_mm) = chassis.design_axle_below_pivot_mm.filter(|&v| v < 0.0) {
        let phi = -axle_offset.atan2(lx_h);
        let arg = (drop_mm / sw_l).clamp(-1.0, 1.0);
        arg.asin() - phi
    } else {
        0.0
    };

    const SAG_FRACTION: f64 = 0.30;
    const GAMMA_PER_STROKE_DEG: f64 = 12.0;
    let gamma_deg = (gamma_design_rad * 180.0 / PI
        + (susp_rr_mm / shock_stroke - SAG_FRACTION) * GAMMA_PER_STROKE_DEG)
        .clamp(-12.0, 12.0);
    let gamma_rad = gamma_deg * PI / 180.0;
    let cg = gamma_rad.cos() as f32;
    let sg = gamma_rad.sin() as f32;

    let body_to_world = |xd: f64, yd: f64| -> (f32, f32) {
        ((xd * cg as f64 - yd * sg as f64) as f32, (xd * sg as f64 + yd * cg as f64) as f32)
    };

    const PITCH_SCALE_DEG_PER_MM: f64 = 0.06;
    let theta_deg = ((susp_fr_mm - susp_rr_mm) * PITCH_SCALE_DEG_PER_MM).clamp(-8.0, 8.0);
    let theta_rad = theta_deg * PI / 180.0;
    let ct = theta_rad.cos() as f32;
    let st = theta_rad.sin() as f32;

    let head_angle_rad = (fr.head_angle_deg * PI / 180.0) as f32;
    let steer_ms = (-head_angle_rad.sin(), -head_angle_rad.cos());
    let steer_dir = (steer_ms.0 * ct - steer_ms.1 * st, steer_ms.0 * st + steer_ms.1 * ct);
    let u_len = (steer_dir.0 * steer_dir.0 + steer_dir.1 * steer_dir.1).sqrt();
    let u = if u_len > 1e-6 { (steer_dir.0 / u_len, steer_dir.1 / u_len) } else { (0.0, -1.0f32) };
    let n = (u.1, -u.0);
    let o = chassis.yoke_offset_mm as f32;

    let axle_offset_world = body_to_world(lx_h, -axle_offset);
    let mut rear_axle = (wb_horiz as f32, r_r as f32);
    let mut pivot = (rear_axle.0 - axle_offset_world.0, rear_axle.1 - axle_offset_world.1);
    let mut front_axle = (0.0f32, r_f as f32);

    if u.1.abs() > 1e-4 {
        for _ in 0..12 {
            let world = |ms_x: f64, ms_y: f64| -> (f32, f32) {
                let vx = (-ms_x) as f32;
                let vy = ms_y as f32;
                let rx = vx * ct - vy * st;
                let ry = vx * st + vy * ct;
                (pivot.0 + rx, pivot.1 + ry)
            };
            let hd = world(fr.head_x, fr.head_y);
            let fb = (hd.0 + o * n.0, hd.1 + o * n.1);
            let s = (r_f as f32 - fb.1) / u.1;
            let nf = (fb.0 + s * u.0, fb.1 + s * u.1);
            if !nf.0.is_finite() || !nf.1.is_finite() { break; }
            let nrear = (nf.0 + wb_horiz as f32, r_r as f32);
            let aow = body_to_world(lx_h, -axle_offset);
            pivot = (nrear.0 - aow.0, nrear.1 - aow.1);
            front_axle = nf;
            rear_axle = nrear;
        }
    }

    let world = |ms_x: f64, ms_y: f64| -> (f32, f32) {
        let vx = (-ms_x) as f32;
        let vy = ms_y as f32;
        let rx = vx * ct - vy * st;
        let ry = vx * st + vy * ct;
        (pivot.0 + rx, pivot.1 + ry)
    };

    let head = world(fr.head_x, fr.head_y);
    let link_mnt = world(fr.link_mnt_x, fr.link_mnt_y);
    let c_shaft = world(fr.c_shaft_x, fr.c_shaft_y);

    // Final refinement pass
    if u.1.abs() > 1e-4 {
        let mut pivot_local = pivot;
        for _ in 0..14 {
            let world2 = |ms_x: f64, ms_y: f64| -> (f32, f32) {
                let vx = (-ms_x) as f32;
                let vy = ms_y as f32;
                let rx = vx * ct - vy * st;
                let ry = vx * st + vy * ct;
                (pivot_local.0 + rx, pivot_local.1 + ry)
            };
            let hd2 = world2(fr.head_x, fr.head_y);
            let fb2 = (hd2.0 + o * n.0, hd2.1 + o * n.1);
            let s2 = (r_f as f32 - fb2.1) / u.1;
            let nf2 = (fb2.0 + s2 * u.0, fb2.1 + s2 * u.1);
            if !nf2.0.is_finite() || !nf2.1.is_finite() { break; }
            let nrear2 = (nf2.0 + wb_horiz as f32, r_r as f32);
            let aow2 = body_to_world(lx_h, -axle_offset);
            let piv2 = (nrear2.0 - aow2.0, nrear2.1 - aow2.1);
            let dist = ((piv2.0 - pivot_local.0).powi(2) + (piv2.1 - pivot_local.1).powi(2)).sqrt();
            pivot_local = piv2;
            front_axle = nf2;
            rear_axle = nrear2;
            if dist < 0.02 { break; }
        }
    }

    let _ = (head, link_mnt, c_shaft);
    let c_shaft_final = world(fr.c_shaft_x, fr.c_shaft_y);
    let link_mnt_final = world(fr.link_mnt_x, fr.link_mnt_y);
    let head_final = world(fr.head_x, fr.head_y);

    draw_common(
        chassis,
        front_axle, rear_axle, pivot, head_final, link_mnt_final, c_shaft_final,
        u, n, o,
        r_f, r_r,
    )
}

// ---------------------------------------------------------------------------
// Shared drawing logic
// ---------------------------------------------------------------------------

fn draw_common(
    chassis: &ChassisModel,
    front_axle: (f32, f32),
    rear_axle: (f32, f32),
    pivot: (f32, f32),
    head: (f32, f32),
    link_mnt: (f32, f32),
    c_shaft: (f32, f32),
    u: (f32, f32),
    n: (f32, f32),
    o: f32,
    r_f: f64,
    r_r: f64,
) -> SideViewSchematic {
    let sw = chassis.swingarm.as_ref();
    let lk = chassis.link.as_ref();
    let fr = chassis.frame.as_ref().unwrap();

    let head_angle_rad = (fr.head_angle_deg * PI / 180.0) as f32;
    let steer_ms = (-head_angle_rad.sin(), -head_angle_rad.cos());
    // steer_dir is reconstructed from u (already normalised)
    let steer_dir = u;

    let mut lines: Vec<SchematicLine> = Vec::new();
    let mut circles: Vec<SchematicCircle> = Vec::new();
    let _ = (steer_ms, n, o);

    let add = |lines: &mut Vec<SchematicLine>, a: (f32, f32), b: (f32, f32), sk: SchematicStroke, col: (u8, u8, u8), th: f64| {
        lines.push(SchematicLine {
            x1: a.0 as f64, y1: a.1 as f64,
            x2: b.0 as f64, y2: b.1 as f64,
            stroke: sk, r: col.0, g: col.1, b: col.2, thickness: th,
        });
    };

    // Ground line
    add(&mut lines, (front_axle.0 - 220.0, 0.0), (rear_axle.0 + 220.0, 0.0),
        SchematicStroke::Solid, COL_GROUND, 1.5);

    // Tires
    let push_circle = |circles: &mut Vec<SchematicCircle>, cx: f32, cy: f32, r: f64, col: (u8, u8, u8), fill: bool, th: f64| {
        circles.push(SchematicCircle { cx: cx as f64, cy: cy as f64, radius_mm: r, r: col.0, g: col.1, b: col.2, fill, thickness: th });
    };

    push_circle(&mut circles, front_axle.0, front_axle.1, r_f, COL_RIM, true, 1.0);
    push_circle(&mut circles, rear_axle.0, rear_axle.1, r_r, COL_RIM, true, 1.0);
    push_circle(&mut circles, front_axle.0, front_axle.1, r_f, COL_TIRE, false, 2.2);
    push_circle(&mut circles, rear_axle.0, rear_axle.1, r_r, COL_TIRE, false, 2.2);
    push_circle(&mut circles, front_axle.0, front_axle.1, r_f * 0.36, COL_HUB, false, 1.4);
    push_circle(&mut circles, rear_axle.0, rear_axle.1, r_r * 0.36, COL_HUB, false, 1.4);
    add_spokes(&mut lines, front_axle, r_f * 0.36, r_f, 4, COL_HUB);
    add_spokes(&mut lines, rear_axle, r_r * 0.36, r_r, 4, COL_HUB);

    // Swingarm
    add(&mut lines, pivot, rear_axle, SchematicStroke::Solid, COL_SWING, 5.0);

    // Frame backbone
    add(&mut lines, pivot, link_mnt, SchematicStroke::Solid, COL_FRAME, 4.0);
    add(&mut lines, link_mnt, head, SchematicStroke::Solid, COL_FRAME, 4.0);
    let head_tube_top = (head.0 - steer_dir.0 * 110.0, head.1 - steer_dir.1 * 110.0);
    add(&mut lines, head, head_tube_top, SchematicStroke::Solid, COL_FRAME, 4.0);
    add(&mut lines, head, c_shaft, SchematicStroke::Solid, COL_FRAME, 2.5);

    // Subframe hint
    let t = 0.45f32;
    let sub_base = lerp2(link_mnt, head, t);
    let sub_end = (sub_base.0 + 280.0, sub_base.1 + 70.0);
    add(&mut lines, sub_base, sub_end, SchematicStroke::Solid, COL_FRAME, 3.0);
    add(&mut lines, sub_end, (sub_end.0 + 90.0, sub_end.1 - 40.0), SchematicStroke::Solid, COL_FRAME, 3.0);

    // Tank hint
    let tank_a = (lerp2(link_mnt, head, 0.3).0, lerp2(link_mnt, head, 0.3).1 + 80.0);
    let tank_b = (lerp2(link_mnt, head, 0.55).0, lerp2(link_mnt, head, 0.55).1 + 130.0);
    let tank_c = (lerp2(link_mnt, head, 0.8).0, lerp2(link_mnt, head, 0.8).1 + 80.0);
    add(&mut lines, tank_a, tank_b, SchematicStroke::Solid, COL_FRAME, 2.0);
    add(&mut lines, tank_b, tank_c, SchematicStroke::Solid, COL_FRAME, 2.0);

    // Fork
    let fork_len_draw = chassis.fork.as_ref()
        .filter(|f| f.length_mm > 50.0)
        .map(|f| f.length_mm as f32)
        .unwrap_or(750.0)
        .min(950.0);
    let fork_upper = (front_axle.0 - steer_dir.0 * fork_len_draw, front_axle.1 - steer_dir.1 * fork_len_draw);
    add(&mut lines, fork_upper, front_axle, SchematicStroke::Solid, COL_FORK, 5.0);

    // Steering axis to ground (dotted)
    if steer_dir.1.abs() > 1e-4 {
        let t_ground = -head.1 / steer_dir.1;
        let steer_foot = (head.0 + steer_dir.0 * t_ground, head.1 + steer_dir.1 * t_ground);
        add(&mut lines, head, steer_foot, SchematicStroke::Dotted, COL_STEERING, 1.6);
    }

    // Front axle plumb
    add(&mut lines, (front_axle.0, 0.0), front_axle, SchematicStroke::Dotted, COL_TRAIL, 1.35);

    // Shock and linkage anchors on swingarm
    if let Some(sw) = sw {
        let shock_below = shock_is_below_body_axis(chassis.link_type.as_deref());
        let gamma_rad_approx = 0.0f32; // approximated as 0 for body_to_world in this shared path
        let cg = gamma_rad_approx.cos();
        let sg = gamma_rad_approx.sin();
        let body_to_world_local = |xd: f32, yd: f32| -> (f32, f32) {
            (xd * cg - yd * sg, xd * sg + yd * cg)
        };
        let sw_shock_rel = body_to_world_local(
            sw.shock_x as f32,
            if shock_below { -(sw.shock_y as f32) } else { sw.shock_y as f32 },
        );
        let sw_shock = (pivot.0 + sw_shock_rel.0, pivot.1 + sw_shock_rel.1);
        add(&mut lines, link_mnt, sw_shock, SchematicStroke::Dashed, COL_SHOCK, 3.0);

        if let Some(lk) = lk {
            if lk.nom_linkarm_l > 1.0 {
                let sw_link_rel = body_to_world_local(sw.link_x as f32, sw.link_y as f32);
                let sw_link = (pivot.0 + sw_link_rel.0, pivot.1 + sw_link_rel.1);
                add(&mut lines, sw_link, link_mnt, SchematicStroke::Dashed, COL_LINK, 1.8);
                push_circle(&mut circles, link_mnt.0, link_mnt.1, 8.0, COL_LINK, false, 1.2);
            }
        }
    }

    // Drive chain
    let pitch_mm = if chassis.chain_pitch_mm > 1.0 { chassis.chain_pitch_mm } else { 15.875 };
    let r_fr_pitch = sprocket_pitch_radius_mm(pitch_mm, chassis.fr_sprocket_teeth);
    let r_rr_pitch = sprocket_pitch_radius_mm(pitch_mm, chassis.rr_sprocket_teeth);
    if r_fr_pitch > 1.0 && r_rr_pitch > 1.0 {
        push_circle(&mut circles, c_shaft.0, c_shaft.1, r_fr_pitch, COL_SPROCKET, false, 1.35);
        push_circle(&mut circles, rear_axle.0, rear_axle.1, r_rr_pitch, COL_SPROCKET, false, 1.35);
        if let Some((up0, up1, lo0, lo1)) = try_chain_external_tangents(
            (c_shaft.0 as f64, c_shaft.1 as f64), r_fr_pitch,
            (rear_axle.0 as f64, rear_axle.1 as f64), r_rr_pitch,
        ) {
            add(&mut lines, (up0.0 as f32, up0.1 as f32), (up1.0 as f32, up1.1 as f32), SchematicStroke::Solid, COL_CHAIN, 2.2);
            add(&mut lines, (lo0.0 as f32, lo0.1 as f32), (lo1.0 as f32, lo1.1 as f32), SchematicStroke::Solid, COL_CHAIN, 2.2);
        }
    }

    // Bounds
    let mut min_x = (front_axle.0 as f64 - r_f).min(head_tube_top.0 as f64) - 30.0;
    let mut max_x = (rear_axle.0 as f64 + r_r).max(sub_end.0 as f64 + 90.0) + 30.0;
    let mut min_y = -40.0_f64;
    let mut max_y = (head_tube_top.1 as f64).max(sub_end.1 as f64).max(tank_b.1 as f64) + 40.0;

    for ln in &lines {
        min_x = min_x.min(ln.x1.min(ln.x2));
        max_x = max_x.max(ln.x1.max(ln.x2));
        min_y = min_y.min(ln.y1.min(ln.y2));
        max_y = max_y.max(ln.y1.max(ln.y2));
    }
    for c in &circles {
        min_x = min_x.min(c.cx - c.radius_mm);
        max_x = max_x.max(c.cx + c.radius_mm);
        min_y = min_y.min(c.cy - c.radius_mm);
        max_y = max_y.max(c.cy + c.radius_mm);
    }
    min_x -= 20.0; max_x += 20.0; min_y -= 10.0; max_y += 30.0;

    let mut legend = vec![
        LegendEntry { label: "Frame".into(), r: COL_FRAME.0, g: COL_FRAME.1, b: COL_FRAME.2, stroke: SchematicStroke::Solid, thickness: 4.0 },
        LegendEntry { label: "Fork".into(), r: COL_FORK.0, g: COL_FORK.1, b: COL_FORK.2, stroke: SchematicStroke::Solid, thickness: 5.0 },
        LegendEntry { label: "Swingarm".into(), r: COL_SWING.0, g: COL_SWING.1, b: COL_SWING.2, stroke: SchematicStroke::Solid, thickness: 5.0 },
        LegendEntry { label: "Shock".into(), r: COL_SHOCK.0, g: COL_SHOCK.1, b: COL_SHOCK.2, stroke: SchematicStroke::Dashed, thickness: 3.0 },
        LegendEntry { label: "Chain".into(), r: COL_CHAIN.0, g: COL_CHAIN.1, b: COL_CHAIN.2, stroke: SchematicStroke::Solid, thickness: 2.2 },
        LegendEntry { label: "Steering axis".into(), r: COL_STEERING.0, g: COL_STEERING.1, b: COL_STEERING.2, stroke: SchematicStroke::Dotted, thickness: 1.6 },
        LegendEntry { label: "Trail / plumb".into(), r: COL_TRAIL.0, g: COL_TRAIL.1, b: COL_TRAIL.2, stroke: SchematicStroke::Dotted, thickness: 1.35 },
    ];
    if lk.map(|l| l.nom_linkarm_l > 1.0).unwrap_or(false) && sw.is_some() {
        legend.push(LegendEntry { label: "Linkage".into(), r: COL_LINK.0, g: COL_LINK.1, b: COL_LINK.2, stroke: SchematicStroke::Dashed, thickness: 1.8 });
    }

    SideViewSchematic {
        lines,
        circles,
        legend,
        bounds: SchematicBounds { min_x, min_y, max_x, max_y },
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn empty() -> SideViewSchematic {
    SideViewSchematic {
        lines: vec![],
        circles: vec![],
        legend: vec![],
        bounds: SchematicBounds { min_x: -10.0, min_y: -10.0, max_x: 10.0, max_y: 10.0 },
    }
}

fn lerp2(a: (f32, f32), b: (f32, f32), t: f32) -> (f32, f32) {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

fn add_spokes(lines: &mut Vec<SchematicLine>, center: (f32, f32), inner_r: f64, outer_r: f64, count: usize, col: (u8, u8, u8)) {
    let r_in = (outer_r * 0.4) as f32;
    let r_out = (outer_r * 0.95) as f32;
    for i in 0..count {
        let ang = i as f64 * PI / count as f64;
        let c = ang.cos() as f32;
        let s = ang.sin() as f32;
        let a = (center.0 - c * r_out, center.1 - s * r_out);
        let b = (center.0 + c * r_out, center.1 + s * r_out);
        let a_gap = (center.0 - c * r_in, center.1 - s * r_in);
        let b_gap = (center.0 + c * r_in, center.1 + s * r_in);
        lines.push(SchematicLine { x1: a.0 as f64, y1: a.1 as f64, x2: a_gap.0 as f64, y2: a_gap.1 as f64, stroke: SchematicStroke::Solid, r: col.0, g: col.1, b: col.2, thickness: 0.8 });
        lines.push(SchematicLine { x1: b_gap.0 as f64, y1: b_gap.1 as f64, x2: b.0 as f64, y2: b.1 as f64, stroke: SchematicStroke::Solid, r: col.0, g: col.1, b: col.2, thickness: 0.8 });
    }
    let _ = inner_r;
}

fn sprocket_pitch_radius_mm(chain_pitch_mm: f64, teeth: i32) -> f64 {
    if teeth < 4 || chain_pitch_mm <= 0.0 { return 0.0; }
    chain_pitch_mm / (2.0 * (PI / teeth as f64).sin())
}

fn try_chain_external_tangents(
    o0: (f64, f64), r0: f64,
    o1: (f64, f64), r1: f64,
) -> Option<((f64, f64), (f64, f64), (f64, f64), (f64, f64))> {
    let wx = o1.0 - o0.0;
    let wy = o1.1 - o0.1;
    let d = (wx * wx + wy * wy).sqrt();
    if d < 1e-3 { return None; }
    let dr = r1 - r0;
    if d <= (dr.abs() - 1e-2) { return None; }

    let beta = wy.atan2(wx);
    let c = (-dr / d).clamp(-1.0, 1.0);
    let delta = c.acos();
    let phi_a = beta + delta;
    let phi_b = beta - delta;
    let na = (phi_a.cos(), phi_a.sin());
    let nb = (phi_b.cos(), phi_b.sin());

    let ta0 = (o0.0 + r0 * na.0, o0.1 + r0 * na.1);
    let ta1 = (o1.0 + r1 * na.0, o1.1 + r1 * na.1);
    let tb0 = (o0.0 + r0 * nb.0, o0.1 + r0 * nb.1);
    let tb1 = (o1.0 + r1 * nb.0, o1.1 + r1 * nb.1);

    if ta0.1 + ta1.1 >= tb0.1 + tb1.1 {
        Some((ta0, ta1, tb0, tb1))
    } else {
        Some((tb0, tb1, ta0, ta1))
    }
}

fn shock_is_below_body_axis(link_type: Option<&str>) -> bool {
    match link_type.map(|t| t.trim().to_ascii_uppercase()).as_deref() {
        Some("PANIGALE") | Some("HORBACKLINK") | Some("LINKLESS") => false,
        _ => true,
    }
}
