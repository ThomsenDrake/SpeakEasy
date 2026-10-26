//! Icons: the exact path data of the reference's Lucide + Once UI (Heroicons
//! outline) icons, rendered as strokes.
//!
//! The reference renders SVGs with `stroke-linecap="round"`
//! `stroke-linejoin="round"` (Lucide stroke-width 2, Heroicons 1.5, viewBox 24).
//! epaint's own path stroke has no cap/join control, so icons are stroked
//! here as a union of convex shapes — one rotated rectangle per segment and
//! one filled circle per vertex — which reproduces round caps and round
//! joins exactly (overlapping same-color shapes are invisible).

use egui::{Color32, Painter, Pos2, Shape, Vec2};

/// Which icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Icon {
    /// Lucide `Slack` (header, 20px).
    Slack,
    /// Lucide `Hash` (destination row, 14px).
    Hash,
    /// Lucide `MessagesSquare` (conversation panel, 16px).
    MessagesSquare,
    /// Once UI `person` (agent tag, 15px).
    Person,
    /// Once UI `chat` (feedback buttons, 15px).
    Chat,
    /// Once UI `close` (Deny button, 15px).
    Close,
    /// Once UI `check` = check-circle (Approve button, 15px).
    Check,
    /// Once UI `inbox` (empty state, 30px).
    Inbox,
}

impl Icon {
    /// The icon's viewBox is 24; the reference's stroke widths in those units.
    fn viewbox_stroke(self) -> f32 {
        match self {
            // Lucide: stroke-width 2.
            Icon::Slack | Icon::Hash | Icon::MessagesSquare => 2.0,
            // Heroicons outline: stroke-width 1.5.
            Icon::Person | Icon::Chat | Icon::Close | Icon::Check | Icon::Inbox => 1.5,
        }
    }

    /// Subpaths (each a polyline in viewBox units, closed or open).
    fn subpaths(self) -> Vec<Subpath> {
        match self {
            Icon::Slack => {
                let mut subs = Vec::new();
                // Four rounded rects (rx = 1.5).
                for (x, y, w, h) in [
                    (13.0, 2.0, 3.0, 8.0),
                    (8.0, 14.0, 3.0, 8.0),
                    (14.0, 13.0, 8.0, 3.0),
                    (2.0, 8.0, 8.0, 3.0),
                ] {
                    subs.push(Subpath {
                        points: rounded_rect(x, y, w, h, 1.5),
                        closed: true,
                    });
                }
                // Four hook paths.
                for d in [
                    "M19 8.5V10h1.5A1.5 1.5 0 1 0 19 8.5",
                    "M5 15.5V14H3.5A1.5 1.5 0 1 0 5 15.5",
                    "M15.5 19H14v1.5a1.5 1.5 0 1 0 1.5-1.5",
                    "M8.5 5H10V3.5A1.5 1.5 0 1 0 8.5 5",
                ] {
                    subs.push(parse_path(d));
                }
                subs
            }
            Icon::Hash => {
                // M4 9h16 M4 15h16 M10 3 8 21 M16 3l-2 18
                vec![
                    Subpath::open(vec![(4.0, 9.0), (20.0, 9.0)]),
                    Subpath::open(vec![(4.0, 15.0), (20.0, 15.0)]),
                    Subpath::open(vec![(10.0, 3.0), (8.0, 21.0)]),
                    Subpath::open(vec![(16.0, 3.0), (14.0, 21.0)]),
                ]
            }
            Icon::MessagesSquare => vec![
                parse_path(
                    "M16 10a2 2 0 0 1-2 2H6.828a2 2 0 0 0-1.414.586l-2.202 2.202A.71.71 0 0 1 2 14.286V4a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2z",
                ),
                parse_path(
                    "M20 9a2 2 0 0 1 2 2v10.286a.71.71 0 0 1-1.212.502l-2.202-2.202A2 2 0 0 0 17.172 19H10a2 2 0 0 1-2-2v-1",
                ),
            ],
            Icon::Person => vec![parse_path(
                "M15.75 6a3.75 3.75 0 1 1-7.5 0 3.75 3.75 0 0 1 7.5 0ZM4.501 20.118a7.5 7.5 0 0 1 14.998 0A17.933 17.933 0 0 1 12 21.75c-2.676 0-5.216-.584-7.499-1.632Z",
            )],
            Icon::Chat => vec![parse_path(
                "M20.25 8.511c.884.284 1.5 1.128 1.5 2.097v4.286c0 1.136-.847 2.1-1.98 2.193-.34.027-.68.052-1.02.072v3.091l-3-3c-1.354 0-2.694-.055-4.02-.163a2.115 2.115 0 0 1-.825-.242m9.345-8.334a2.126 2.126 0 0 0-.476-.095 48.64 48.64 0 0 0-8.048 0c-1.131.094-1.976 1.057-1.976 2.192v4.286c0 .837.46 1.58 1.155 1.951m9.345-8.334V6.637c0-1.621-1.152-3.026-2.76-3.235A48.455 48.455 0 0 0 11.25 3c-2.115 0-4.198.137-6.24.402-1.608.209-2.76 1.614-2.76 3.235v6.226c0 1.621 1.152 3.026 2.76 3.235.577.075 1.157.14 1.74.194V21l4.155-4.155",
            )],
            Icon::Close => vec![Subpath::open(vec![(6.0, 18.0), (18.0, 6.0)]), Subpath::open(vec![(6.0, 6.0), (18.0, 18.0)])],
            Icon::Check => vec![parse_path(
                "M9 12.75 11.25 15 15 9.75M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0Z",
            )],
            Icon::Inbox => vec![parse_path(
                "M2.25 13.5h3.86a2.25 2.25 0 0 1 2.012 1.244l.256.512a2.25 2.25 0 0 0 2.013 1.244h3.218a2.25 2.25 0 0 0 2.013-1.244l.256-.512a2.25 2.25 0 0 1 2.013-1.244h3.859m-19.5.338V18a2.25 2.25 0 0 0 2.25 2.25h15A2.25 2.25 0 0 0 21.75 18v-4.162c0-.224-.034-.447-.1-.661L19.24 5.338a2.25 2.25 0 0 0-2.15-1.588H6.911a2.25 2.25 0 0 0-2.15 1.588L2.35 13.177a2.25 2.25 0 0 0-.1.661Z",
            )],
        }
    }
}

struct Subpath {
    points: Vec<(f32, f32)>,
    closed: bool,
}

impl Subpath {
    fn open(points: Vec<(f32, f32)>) -> Self {
        Self {
            points,
            closed: false,
        }
    }
}

/// A rounded rectangle outline (clockwise), as a closed polyline.
fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Vec<(f32, f32)> {
    let r = r.min(w / 2.0).min(h / 2.0);
    let mut pts = Vec::new();
    let arc = |cx: f32, cy: f32, a0: f32| {
        (0..=8)
            .map(|i| {
                let a = a0 + (i as f32 / 8.0) * std::f32::consts::FRAC_PI_2;
                (cx + r * a.cos(), cy + r * a.sin())
            })
            .collect::<Vec<_>>()
    };
    // Start at (x + r, y), go clockwise: top edge, top-right arc, right edge, …
    pts.push((x + r, y));
    pts.push((x + w - r, y));
    pts.extend(arc(x + w - r, y + r, -std::f32::consts::FRAC_PI_2));
    pts.push((x + w, y + h - r));
    pts.extend(arc(x + w - r, y + h - r, 0.0));
    pts.push((x + r, y + h));
    pts.extend(arc(x + r, y + h - r, std::f32::consts::FRAC_PI_2));
    pts.push((x, y + r));
    pts.extend(arc(x + r, y + r, std::f32::consts::PI));
    pts
}

// ---------------------------------------------------------------------------
// Minimal SVG path-data parser (M/L/H/V/C/S/Q/T/A/Z, absolute & relative),
// flattening curves and arcs to polylines.
// ---------------------------------------------------------------------------

fn parse_path(d: &str) -> Subpath {
    let mut p = PathParser {
        tokens: d,
        pos: 0,
    };
    let mut subpaths: Vec<Subpath> = Vec::new();
    let mut cur: Vec<(f32, f32)> = Vec::new();
    let mut start = (0.0, 0.0);
    let mut pos = (0.0, 0.0);
    let mut last_ctrl: Option<(f32, f32)> = None;
    let mut cmd = ' ';
    let mut closed = false;

    macro_rules! flush {
        () => {
            if cur.len() >= 2 {
                subpaths.push(Subpath {
                    points: std::mem::take(&mut cur),
                    closed,
                });
            } else {
                cur.clear();
            }
        };
    }

    loop {
        p.skip_ws();
        if p.pos >= p.tokens.len() {
            break;
        }
        let c = p.tokens[p.pos..].chars().next().unwrap();
        if c.is_ascii_alphabetic() {
            cmd = c;
            p.pos += c.len_utf8();
        }
        let rel = cmd.is_ascii_lowercase();
        let ucmd = cmd.to_ascii_uppercase();
        match ucmd {
            'M' => {
                flush!();
                // A new subpath starts open (the previous subpath's `closed`
                // was consumed by its flush).
                closed = false;
                let (x, y) = p.num_pair();
                let pt = if rel {
                    (pos.0 + x, pos.1 + y)
                } else {
                    (x, y)
                };
                pos = pt;
                start = pt;
                cur.push(pt);
                // Implicit lineto after moveto.
                cmd = if rel { 'l' } else { 'L' };
            }
            'L' => {
                let (x, y) = p.num_pair();
                let pt = if rel {
                    (pos.0 + x, pos.1 + y)
                } else {
                    (x, y)
                };
                pos = pt;
                cur.push(pt);
            }
            'H' => {
                let x = p.num();
                pos.0 = if rel { pos.0 + x } else { x };
                cur.push(pos);
            }
            'V' => {
                let y = p.num();
                pos.1 = if rel { pos.1 + y } else { y };
                cur.push(pos);
            }
            'C' => {
                let (x1, y1) = p.num_pair();
                let (x2, y2) = p.num_pair();
                let (x, y) = p.num_pair();
                let (c1, c2, end) = if rel {
                    (
                        (pos.0 + x1, pos.1 + y1),
                        (pos.0 + x2, pos.1 + y2),
                        (pos.0 + x, pos.1 + y),
                    )
                } else {
                    ((x1, y1), (x2, y2), (x, y))
                };
                flatten_cubic(&mut cur, pos, c1, c2, end);
                last_ctrl = Some(c2);
                pos = end;
            }
            'S' => {
                let (x2, y2) = p.num_pair();
                let (x, y) = p.num_pair();
                let c1 = match last_ctrl {
                    Some(c) => (2.0 * pos.0 - c.0, 2.0 * pos.1 - c.1),
                    None => pos,
                };
                let (c2, end) = if rel {
                    ((pos.0 + x2, pos.1 + y2), (pos.0 + x, pos.1 + y))
                } else {
                    ((x2, y2), (x, y))
                };
                flatten_cubic(&mut cur, pos, c1, c2, end);
                last_ctrl = Some(c2);
                pos = end;
            }
            'Q' => {
                let (x1, y1) = p.num_pair();
                let (x, y) = p.num_pair();
                let (c1, end) = if rel {
                    ((pos.0 + x1, pos.1 + y1), (pos.0 + x, pos.1 + y))
                } else {
                    ((x1, y1), (x, y))
                };
                flatten_quad(&mut cur, pos, c1, end);
                last_ctrl = Some(c1);
                pos = end;
            }
            'T' => {
                let (x, y) = p.num_pair();
                let c1 = match last_ctrl {
                    Some(c) => (2.0 * pos.0 - c.0, 2.0 * pos.1 - c.1),
                    None => pos,
                };
                let end = if rel {
                    (pos.0 + x, pos.1 + y)
                } else {
                    (x, y)
                };
                flatten_quad(&mut cur, pos, c1, end);
                last_ctrl = Some(c1);
                pos = end;
            }
            'A' => {
                let rx = p.num();
                let ry = p.num();
                let rot = p.num();
                let large = p.flag();
                let sweep = p.flag();
                let (x, y) = p.num_pair();
                let end = if rel {
                    (pos.0 + x, pos.1 + y)
                } else {
                    (x, y)
                };
                flatten_arc(&mut cur, pos, rx, ry, rot, large != 0.0, sweep != 0.0, end);
                pos = end;
                last_ctrl = None;
            }
            'Z' => {
                closed = true;
                if let Some(&first) = cur.first() {
                    if first != pos {
                        cur.push(first);
                    }
                }
                pos = start;
                last_ctrl = None;
                p.skip_ws();
                if p.pos < p.tokens.len() {
                    let c2 = p.tokens[p.pos..].chars().next().unwrap();
                    if c2.is_ascii_alphabetic() {
                        continue;
                    }
                }
                flush!();
                cmd = ' ';
            }
            other => {
                panic!("unsupported SVG path command: {other}");
            }
        }
    }
    flush!();
    // The reference icons are single-subpath; merge defensively.
    let mut points = Vec::new();
    let mut closed = false;
    for s in subpaths {
        points.extend(s.points);
        closed = closed || s.closed;
    }
    Subpath { points, closed }
}

struct PathParser<'a> {
    tokens: &'a str,
    pos: usize,
}

impl<'a> PathParser<'a> {
    fn skip_ws(&mut self) {
        while self.pos < self.tokens.len() {
            let c = self.tokens[self.pos..].chars().next().unwrap();
            if c.is_ascii_whitespace() || c == ',' {
                self.pos += c.len_utf8();
            } else {
                break;
            }
        }
    }

    /// Parse one SVG number: `[+-]? ( digits [. digits?]? | . digits ) ( [eE] [+-]? digits )?`.
    /// A sign or `e` only continues the number where the grammar allows it, so
    /// `1.5-1.5` reads as two numbers (the `-` starts the next one).
    fn num(&mut self) -> f32 {
        self.skip_ws();
        let bytes = self.tokens.as_bytes();
        let start = self.pos;
        let mut pos = start;
        if pos < bytes.len() && (bytes[pos] == b'-' || bytes[pos] == b'+') {
            pos += 1;
        }
        let int_start = pos;
        while pos < bytes.len() && bytes[pos].is_ascii_digit() {
            pos += 1;
        }
        let int_digits = pos - int_start;
        let mut frac_digits = 0;
        if pos < bytes.len() && bytes[pos] == b'.' {
            pos += 1;
            let frac_start = pos;
            while pos < bytes.len() && bytes[pos].is_ascii_digit() {
                pos += 1;
            }
            frac_digits = pos - frac_start;
        }
        if int_digits == 0 && frac_digits == 0 {
            panic!("bad number in path data at {start}: {:?}", self.tokens);
        }
        if pos < bytes.len() && (bytes[pos] == b'e' || bytes[pos] == b'E') {
            let mut epos = pos + 1;
            if epos < bytes.len() && (bytes[epos] == b'-' || bytes[epos] == b'+') {
                epos += 1;
            }
            let ed_start = epos;
            while epos < bytes.len() && bytes[epos].is_ascii_digit() {
                epos += 1;
            }
            if epos > ed_start {
                pos = epos;
            }
        }
        let text = &self.tokens[start..pos];
        self.pos = pos;
        text.parse()
            .unwrap_or_else(|_| panic!("bad number in path data at {start}: {:?}", self.tokens))
    }

    /// Arc flags are single `0`/`1` characters and may be concatenated without
    /// separators (`…0 11…`), so they cannot go through `num`.
    fn flag(&mut self) -> f32 {
        self.skip_ws();
        let start = self.pos;
        if self.pos < self.tokens.len() {
            let c = self.tokens[self.pos..].chars().next().unwrap();
            if c == '0' || c == '1' {
                self.pos += 1;
                return if c == '0' { 0.0 } else { 1.0 };
            }
        }
        panic!("bad arc flag in path data at {start}: {:?}", self.tokens);
    }

    fn num_pair(&mut self) -> (f32, f32) {
        let x = self.num();
        let y = self.num();
        (x, y)
    }
}

fn flatten_cubic(
    out: &mut Vec<(f32, f32)>,
    p0: (f32, f32),
    p1: (f32, f32),
    p2: (f32, f32),
    p3: (f32, f32),
) {
    const STEPS: usize = 16;
    for i in 1..=STEPS {
        let t = i as f32 / STEPS as f32;
        let mt = 1.0 - t;
        let x = mt * mt * mt * p0.0
            + 3.0 * mt * mt * t * p1.0
            + 3.0 * mt * t * t * p2.0
            + t * t * t * p3.0;
        let y = mt * mt * mt * p0.1
            + 3.0 * mt * mt * t * p1.1
            + 3.0 * mt * t * t * p2.1
            + t * t * t * p3.1;
        out.push((x, y));
    }
}

fn flatten_quad(out: &mut Vec<(f32, f32)>, p0: (f32, f32), p1: (f32, f32), p2: (f32, f32)) {
    const STEPS: usize = 12;
    for i in 1..=STEPS {
        let t = i as f32 / STEPS as f32;
        let mt = 1.0 - t;
        let x = mt * mt * p0.0 + 2.0 * mt * t * p1.0 + t * t * p2.0;
        let y = mt * mt * p0.1 + 2.0 * mt * t * p1.1 + t * t * p2.1;
        out.push((x, y));
    }
}

#[allow(clippy::too_many_arguments)]
fn flatten_arc(
    out: &mut Vec<(f32, f32)>,
    p0: (f32, f32),
    rx: f32,
    ry: f32,
    x_axis_rot_deg: f32,
    large_arc: bool,
    sweep: bool,
    p1: (f32, f32),
) {
    // SVG endpoint→center parameterization (F.6.5).
    let rx = rx.abs().max(1e-6);
    let ry = ry.abs().max(1e-6);
    let phi = x_axis_rot_deg.to_radians();
    let (cos_p, sin_p) = (phi.cos(), phi.sin());
    let dx = (p0.0 - p1.0) / 2.0;
    let dy = (p0.1 - p1.1) / 2.0;
    let x1p = cos_p * dx + sin_p * dy;
    let y1p = -sin_p * dx + cos_p * dy;
    let lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
    let (rx, ry) = if lambda > 1.0 {
        let s = lambda.sqrt();
        (rx * s, ry * s)
    } else {
        (rx, ry)
    };
    let num = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p;
    let den = rx * rx * y1p * y1p + ry * ry * x1p * x1p;
    let co = (num / den.max(1e-9)).sqrt().max(0.0);
    let sign = if large_arc == sweep { -1.0 } else { 1.0 };
    let cxp = sign * co * (rx * y1p / ry);
    let cyp = sign * co * -(ry * x1p / rx);
    let cx = cos_p * cxp - sin_p * cyp + (p0.0 + p1.0) / 2.0;
    let cy = sin_p * cxp + cos_p * cyp + (p0.1 + p1.1) / 2.0;
    let theta = |ux: f32, uy: f32, vx: f32, vy: f32| {
        let dot = ux * vx + uy * vy;
        let len = (ux * ux + uy * uy).sqrt() * (vx * vx + vy * vy).sqrt();
        let mut a = (dot / len.max(1e-9)).clamp(-1.0, 1.0).acos();
        if ux * vy - uy * vx < 0.0 {
            a = -a;
        }
        a
    };
    let th1 = theta(1.0, 0.0, (x1p - cxp) / rx, (y1p - cyp) / ry);
    let mut dth = theta(
        (x1p - cxp) / rx,
        (y1p - cyp) / ry,
        (-x1p - cxp) / rx,
        (-y1p - cyp) / ry,
    );
    if !sweep && dth > 0.0 {
        dth -= std::f32::consts::TAU;
    } else if sweep && dth < 0.0 {
        dth += std::f32::consts::TAU;
    }
    let steps = ((dth.abs() / (std::f32::consts::TAU / 24.0)).ceil() as usize).clamp(4, 64);
    for i in 1..=steps {
        let t = th1 + dth * (i as f32 / steps as f32);
        let (ct, st) = (t.cos(), t.sin());
        out.push((
            cos_p * rx * ct - sin_p * ry * st + cx,
            sin_p * rx * ct + cos_p * ry * st + cy,
        ));
    }
}

// ---------------------------------------------------------------------------
// Round-cap/round-join stroking as a union of convex shapes.
// ---------------------------------------------------------------------------

/// Paint `icon` at `size`×`size` with top-left `pos`, in `color`.
pub fn paint_icon(painter: &Painter, icon: Icon, pos: Pos2, size: f32, color: Color32) {
    let scale = size / 24.0;
    let stroke_w = icon.viewbox_stroke() * scale;
    let half = stroke_w / 2.0;
    let origin = pos.to_vec2();
    let to_px = |p: (f32, f32)| -> Pos2 {
        Pos2::new(origin.x + p.0 * scale, origin.y + p.1 * scale)
    };

    for sub in icon.subpaths() {
        let pts: Vec<Pos2> = sub.points.iter().map(|&p| to_px(p)).collect();
        if pts.len() < 2 {
            continue;
        }
        let n = pts.len();
        let last = if sub.closed { n } else { n - 1 };
        for i in 0..last {
            let a = pts[i];
            let b = pts[(i + 1) % n];
            let d = b - a;
            let len = d.length();
            if len < 1e-6 {
                continue;
            }
            let normal = Vec2::new(-d.y / len, d.x / len) * half;
            let quad = vec![a + normal, b + normal, b - normal, a - normal];
            painter.add(Shape::convex_polygon(quad, color, egui::Stroke::NONE));
        }
        // Round joins at every vertex; for open subpaths the first/last
        // vertices double as round end caps.
        for &v in &pts {
            painter.add(Shape::circle_filled(v, half, color));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_icon_paths() {
        for icon in [
            Icon::Slack,
            Icon::Hash,
            Icon::MessagesSquare,
            Icon::Person,
            Icon::Chat,
            Icon::Close,
            Icon::Check,
            Icon::Inbox,
        ] {
            let subs = icon.subpaths();
            assert!(!subs.is_empty(), "{icon:?} has no subpaths");
            for s in &subs {
                assert!(s.points.len() >= 2, "{icon:?} subpath too short");
                for &(x, y) in &s.points {
                    assert!(x.is_finite() && y.is_finite(), "{icon:?} non-finite point");
                    assert!(
                        (-2.0..=26.0).contains(&x) && (-2.0..=26.0).contains(&y),
                        "{icon:?} point out of viewBox: ({x}, {y})"
                    );
                }
            }
        }
    }

    #[test]
    fn rounded_rect_stays_in_bounds() {
        let pts = rounded_rect(13.0, 2.0, 3.0, 8.0, 1.5);
        for &(x, y) in &pts {
            assert!((13.0..=16.0).contains(&x), "x {x}");
            assert!((2.0..=10.0).contains(&y), "y {y}");
        }
    }
}
