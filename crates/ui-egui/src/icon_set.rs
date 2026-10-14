//! The UI design's icon set (`docs/soundcraft-ai/design/icons.json`, board 6c): 63 icons drawn from
//! path data at 20 px and 16 px, 1 px round strokes, filled shapes for four of them. The data is a copy
//! in `assets/icons.json` (a test keeps it identical to the design's). Paths use only
//! `M L H V A C S Z`, absolute and relative; they are flattened to polylines once, at first use.
//!
//! Nothing here can panic on bad data: a path that does not parse gives no icon, and `draw` then
//! reports `false` so the caller can use another drawing.

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, pos2};
use std::collections::HashMap;
use std::sync::OnceLock;

/// A connected run of points, closed when the path said `z`.
#[derive(Debug, Clone, PartialEq)]
pub struct Subpath {
    pub points: Vec<[f32; 2]>,
    pub closed: bool,
}

/// One icon at one size: the stroked outline and the filled shapes, in grid units.
#[derive(Debug, Clone, Default)]
pub struct SizedIcon {
    pub stroke: Vec<Subpath>,
    pub fill: Vec<Subpath>,
}

#[derive(Debug, Clone, Default)]
pub struct Icon {
    pub size_20: SizedIcon,
    pub size_16: SizedIcon,
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl Reader<'_> {
    fn skip_separators(&mut self) {
        while self.b.get(self.i).is_some_and(|c| matches!(c, b' ' | b',' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }

    fn command(&mut self) -> Option<u8> {
        self.skip_separators();
        let c = *self.b.get(self.i)?;
        if c.is_ascii_alphabetic() {
            self.i += 1;
            Some(c)
        } else {
            None
        }
    }

    fn at_number(&mut self) -> bool {
        self.skip_separators();
        self.b.get(self.i).is_some_and(|c| c.is_ascii_digit() || matches!(c, b'-' | b'+' | b'.'))
    }

    fn number(&mut self) -> Option<f32> {
        self.skip_separators();
        let start = self.i;
        if matches!(self.b.get(self.i), Some(b'-' | b'+')) {
            self.i += 1;
        }
        let mut digits = 0;
        while self.b.get(self.i).is_some_and(u8::is_ascii_digit) {
            self.i += 1;
            digits += 1;
        }
        if self.b.get(self.i) == Some(&b'.') {
            self.i += 1;
            while self.b.get(self.i).is_some_and(u8::is_ascii_digit) {
                self.i += 1;
                digits += 1;
            }
        }
        if digits == 0 {
            return None;
        }
        if matches!(self.b.get(self.i), Some(b'e' | b'E')) {
            let save = self.i;
            self.i += 1;
            if matches!(self.b.get(self.i), Some(b'-' | b'+')) {
                self.i += 1;
            }
            if self.b.get(self.i).is_some_and(u8::is_ascii_digit) {
                while self.b.get(self.i).is_some_and(u8::is_ascii_digit) {
                    self.i += 1;
                }
            } else {
                self.i = save;
            }
        }
        let v: f32 = std::str::from_utf8(self.b.get(start..self.i)?).ok()?.parse().ok()?;
        v.is_finite().then_some(v)
    }

    /// An arc flag is one character, `0` or `1`, and may be glued to the next number.
    fn flag(&mut self) -> Option<bool> {
        self.skip_separators();
        let c = *self.b.get(self.i)?;
        self.i += 1;
        match c {
            b'0' => Some(false),
            b'1' => Some(true),
            _ => None,
        }
    }
}

/// Points of an SVG arc from `from` to `to`, per the specification's endpoint parameterisation.
fn arc_points(from: [f32; 2], to: [f32; 2], mut rx: f32, mut ry: f32, rot_deg: f32, large: bool, sweep: bool) -> Vec<[f32; 2]> {
    if from == to {
        return Vec::new();
    }
    if rx.abs() < 1e-6 || ry.abs() < 1e-6 {
        return vec![to];
    }
    rx = rx.abs();
    ry = ry.abs();
    let (phi_sin, phi_cos) = rot_deg.to_radians().sin_cos();
    let (dx, dy) = ((from[0] - to[0]) / 2.0, (from[1] - to[1]) / 2.0);
    let (x1, y1) = (phi_cos * dx + phi_sin * dy, -phi_sin * dx + phi_cos * dy);
    let lambda = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if lambda > 1.0 {
        let s = lambda.sqrt();
        rx *= s;
        ry *= s;
    }
    let num = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
    let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let sign = if large == sweep { -1.0 } else { 1.0 };
    let k = if den.abs() < 1e-12 { 0.0 } else { sign * (num / den).max(0.0).sqrt() };
    let (cxp, cyp) = (k * rx * y1 / ry, -k * ry * x1 / rx);
    let (cx, cy) = (phi_cos * cxp - phi_sin * cyp + (from[0] + to[0]) / 2.0, phi_sin * cxp + phi_cos * cyp + (from[1] + to[1]) / 2.0);
    let angle = |ux: f32, uy: f32, vx: f32, vy: f32| (ux * vy - uy * vx).atan2(ux * vx + uy * vy);
    let theta1 = angle(1.0, 0.0, (x1 - cxp) / rx, (y1 - cyp) / ry);
    let mut delta = angle((x1 - cxp) / rx, (y1 - cyp) / ry, (-x1 - cxp) / rx, (-y1 - cyp) / ry);
    if !sweep && delta > 0.0 {
        delta -= std::f32::consts::TAU;
    } else if sweep && delta < 0.0 {
        delta += std::f32::consts::TAU;
    }
    let steps = ((delta.abs() / (std::f32::consts::FRAC_PI_2)) * 8.0).ceil().clamp(2.0, 64.0) as usize;
    let mut pts: Vec<[f32; 2]> = (1..=steps)
        .map(|i| {
            let t = theta1 + delta * i as f32 / steps as f32;
            let (st, ct) = t.sin_cos();
            [phi_cos * rx * ct - phi_sin * ry * st + cx, phi_sin * rx * ct + phi_cos * ry * st + cy]
        })
        .collect();
    // Rounding must not move the end point off the one the path named.
    if let Some(last) = pts.last_mut() {
        *last = to;
    }
    pts
}

fn cubic_points(p0: [f32; 2], p1: [f32; 2], p2: [f32; 2], p3: [f32; 2]) -> Vec<[f32; 2]> {
    (1..=12)
        .map(|i| {
            let t = i as f32 / 12.0;
            let u = 1.0 - t;
            let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
            [w[0] * p0[0] + w[1] * p1[0] + w[2] * p2[0] + w[3] * p3[0], w[0] * p0[1] + w[1] * p1[1] + w[2] * p2[1] + w[3] * p3[1]]
        })
        .collect()
}

/// Parse and flatten an SVG path into subpaths. `None` for anything malformed.
pub fn parse(d: &str) -> Option<Vec<Subpath>> {
    let mut r = Reader { b: d.as_bytes(), i: 0 };
    let mut out: Vec<Subpath> = Vec::new();
    let (mut cur, mut start) = ([0.0f32; 2], [0.0f32; 2]);
    let mut last_ctrl: Option<[f32; 2]> = None;
    let mut cmd = 0u8;
    let mut open: Option<Subpath> = None;
    let finish = |open: &mut Option<Subpath>, out: &mut Vec<Subpath>| {
        if let Some(s) = open.take().filter(|s| s.points.len() > 1) {
            out.push(s);
        }
    };
    loop {
        r.skip_separators();
        if r.i >= r.b.len() {
            break;
        }
        if let Some(c) = r.command() {
            cmd = c;
        } else if !r.at_number() || cmd == 0 || matches!(cmd, b'z' | b'Z') {
            // Numbers with no command, or numbers after a close, are not a path.
            return None;
        }
        let rel = cmd.is_ascii_lowercase();
        let base = if rel { cur } else { [0.0, 0.0] };
        match cmd.to_ascii_uppercase() {
            b'M' => {
                finish(&mut open, &mut out);
                let (x, y) = (r.number()?, r.number()?);
                cur = [base[0] + x, base[1] + y];
                start = cur;
                open = Some(Subpath { points: vec![cur], closed: false });
                last_ctrl = None;
                // Further pairs after a move are implicit lines.
                cmd = if rel { b'l' } else { b'L' };
            }
            b'L' => {
                let (x, y) = (r.number()?, r.number()?);
                cur = [base[0] + x, base[1] + y];
                open.as_mut()?.points.push(cur);
                last_ctrl = None;
            }
            b'H' => {
                let x = r.number()?;
                cur = [if rel { cur[0] + x } else { x }, cur[1]];
                open.as_mut()?.points.push(cur);
                last_ctrl = None;
            }
            b'V' => {
                let y = r.number()?;
                cur = [cur[0], if rel { cur[1] + y } else { y }];
                open.as_mut()?.points.push(cur);
                last_ctrl = None;
            }
            b'A' => {
                let (rx, ry, rot) = (r.number()?, r.number()?, r.number()?);
                let (large, sweep) = (r.flag()?, r.flag()?);
                let (x, y) = (r.number()?, r.number()?);
                let to = [base[0] + x, base[1] + y];
                open.as_mut()?.points.extend(arc_points(cur, to, rx, ry, rot, large, sweep));
                cur = to;
                last_ctrl = None;
            }
            b'C' => {
                let mut v = [0.0f32; 6];
                for slot in &mut v {
                    *slot = r.number()?;
                }
                let (c1, c2, to) = ([base[0] + v[0], base[1] + v[1]], [base[0] + v[2], base[1] + v[3]], [base[0] + v[4], base[1] + v[5]]);
                open.as_mut()?.points.extend(cubic_points(cur, c1, c2, to));
                cur = to;
                last_ctrl = Some(c2);
            }
            b'S' => {
                let mut v = [0.0f32; 4];
                for slot in &mut v {
                    *slot = r.number()?;
                }
                let c1 = last_ctrl.map_or(cur, |c| [2.0 * cur[0] - c[0], 2.0 * cur[1] - c[1]]);
                let (c2, to) = ([base[0] + v[0], base[1] + v[1]], [base[0] + v[2], base[1] + v[3]]);
                open.as_mut()?.points.extend(cubic_points(cur, c1, c2, to));
                cur = to;
                last_ctrl = Some(c2);
            }
            b'Z' => {
                if let Some(mut s) = open.take() {
                    s.closed = true;
                    if s.points.len() > 2 {
                        out.push(s);
                    }
                }
                cur = start;
                last_ctrl = None;
                // A path may continue after z with a command that starts from the subpath start.
                open = Some(Subpath { points: vec![cur], closed: false });
            }
            _ => return None,
        }
        if out.len() > 64 || open.as_ref().is_some_and(|s| s.points.len() > 4096) {
            return None;
        }
    }
    finish(&mut open, &mut out);
    out.iter().all(|s| s.points.iter().all(|p| p[0].is_finite() && p[1].is_finite())).then_some(out)
}

fn sized(stroke: &str, fill: &str) -> Option<SizedIcon> {
    let part = |d: &str| if d.trim().is_empty() || d.trim() == "M0 0" { Some(Vec::new()) } else { parse(d) };
    Some(SizedIcon { stroke: part(stroke)?, fill: part(fill)? })
}

fn load() -> HashMap<String, Icon> {
    let mut map = HashMap::new();
    let Ok(v) = serde_json::from_str::<serde_json::Value>(include_str!("../assets/icons.json")) else {
        log::error!("icons.json is not valid JSON");
        return map;
    };
    for i in v["icons"].as_array().map(Vec::as_slice).unwrap_or_default() {
        let text = |a: &str, b: &str| i[a][b].as_str().unwrap_or("").to_string();
        let (Some(name), Some(s20), Some(s16)) = (
            i["name"].as_str(),
            sized(&text("size_20", "stroke"), &text("size_20", "fill")),
            sized(&text("size_16", "stroke"), &text("size_16", "fill")),
        ) else {
            log::warn!("an icon in icons.json did not parse: {}", i["name"]);
            continue;
        };
        map.insert(name.to_string(), Icon { size_20: s20, size_16: s16 });
    }
    map
}

fn set() -> &'static HashMap<String, Icon> {
    static SET: OnceLock<HashMap<String, Icon>> = OnceLock::new();
    SET.get_or_init(load)
}

pub fn get(name: &str) -> Option<&'static Icon> {
    set().get(name)
}

/// Names the older procedural icons used, mapped to the design's.
fn alias(name: &str) -> &str {
    match name {
        "rtz" => "to-start",
        "ffwd" => "fast-forward",
        "gear" => "settings",
        "wave" => "waveform-view",
        other => other,
    }
}

/// Draw icon `name` in `r`: the 16 px drawing up to 18 px, otherwise the 20 px drawing scaled to fit.
/// Returns `false` when the design has no such icon, so the caller can draw another.
pub fn draw(painter: &Painter, r: Rect, name: &str, color: Color32) -> bool {
    let Some(icon) = get(alias(name)) else { return false };
    let side = r.width().min(r.height());
    let (art, grid) = if side <= 18.0 { (&icon.size_16, 16.0) } else { (&icon.size_20, 20.0) };
    let scale = side / grid;
    // The design offsets the artwork by half a unit so a 1 unit stroke is centred on a pixel.
    let at = |p: &[f32; 2]| -> Pos2 { pos2(r.center().x - side / 2.0 + (p[0] + 0.5) * scale, r.center().y - side / 2.0 + (p[1] + 0.5) * scale) };
    let stroke = Stroke::new(scale.max(1.0), color);
    for s in &art.stroke {
        let pts: Vec<Pos2> = s.points.iter().map(at).collect();
        painter.add(if s.closed { Shape::closed_line(pts, stroke) } else { Shape::line(pts, stroke) });
    }
    for s in &art.fill {
        let pts: Vec<Pos2> = s.points.iter().map(at).collect();
        painter.add(Shape::convex_polygon(pts, color, stroke));
    }
    true
}

#[cfg(test)]
mod tests;
