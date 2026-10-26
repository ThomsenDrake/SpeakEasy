//! Animation: faithful ports of the three effects the reference UI uses.
//!
//! - [`Spring`]: motion-dom's physics spring — the same analytic solution,
//!   rest detection and retargeting as `useSpring` (used by `AnimatedNumber`
//!   with the physics defaults stiffness=100/damping=10/mass=1).
//! - [`TextEffect`]: the per-word `fade-in-blur` reveal — a *tween*
//!   (0.3s, stagger 0.05s/segment), not a spring.
//! - [`AnimatedNumber`]: the pending-count badge — spring interpolation
//!   toward the queue length, rounded and locale-formatted.
//!
//! All animations are driven by the frame clock (`egui::Context` time) and
//! stop doing work once settled, hidden, or reduced-motion is requested.

/// A CSS cubic-bezier easing curve, evaluated the way browsers do
/// (Newton-Raphson on the x curve, with bisection fallback).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CubicBezier {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

impl CubicBezier {
    /// motion's named `easeOut` (`cubic-bezier(0, 0, 0.58, 1)`) — the default
    /// ease of `animateMotionValue` when a variant defines a transition with
    /// a duration but no ease (the TextEffect item variants).
    pub const EASE_OUT: Self = Self {
        x1: 0.0,
        y1: 0.0,
        x2: 0.58,
        y2: 1.0,
    };
    /// motion-dom's `getDefaultTransition` ease for non-transform values
    /// (`[0.25, 0.1, 0.35, 1]`, duration 0.3s) — used by the TextEffect
    /// container variant, whose transition has only orchestration keys.
    pub const MOTION_DEFAULT: Self = Self {
        x1: 0.25,
        y1: 0.1,
        x2: 0.35,
        y2: 1.0,
    };
    /// CSS `ease-in-out` (`cubic-bezier(0.42, 0, 0.58, 1)`) — Once UI's
    /// `--transition-micro-medium` (`all 0.2s ease-in-out`) applied to
    /// interactive controls.
    pub const EASE_IN_OUT: Self = Self {
        x1: 0.42,
        y1: 0.0,
        x2: 0.58,
        y2: 1.0,
    };

    pub fn eval(&self, t: f32) -> f32 {
        if t <= 0.0 {
            return 0.0;
        }
        if t >= 1.0 {
            return 1.0;
        }
        // Solve bezier_x(s) = t for s.
        let (x1, y1, x2, y2) = (self.x1, self.y1, self.x2, self.y2);
        let cx = 3.0 * x1;
        let bx = 3.0 * (x2 - x1) - cx;
        let ax = 1.0 - cx - bx;
        let cy = 3.0 * y1;
        let by = 3.0 * (y2 - y1) - cy;
        let ay = 1.0 - cy - by;
        let sample_x = |s: f32| ((ax * s + bx) * s + cx) * s;
        let sample_y = |s: f32| ((ay * s + by) * s + cy) * s;
        let deriv_x = |s: f32| (3.0 * ax * s + 2.0 * bx) * s + cx;

        let mut s = t;
        for _ in 0..8 {
            let x = sample_x(s) - t;
            if x.abs() < 1e-6 {
                return sample_y(s);
            }
            let d = deriv_x(s);
            if d.abs() < 1e-6 {
                break;
            }
            s -= x / d;
            if !(0.0..=1.0).contains(&s) {
                break;
            }
        }
        // Bisection fallback.
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        let mut s = t;
        for _ in 0..24 {
            let x = sample_x(s);
            if (x - t).abs() < 1e-6 {
                break;
            }
            if x < t {
                lo = s;
            } else {
                hi = s;
            }
            s = (lo + hi) / 2.0;
        }
        sample_y(s)
    }
}

// ---------------------------------------------------------------------------
// Spring (motion-dom port)
// ---------------------------------------------------------------------------

/// Spring options; the physics defaults are motion-dom's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpringOptions {
    pub stiffness: f32,
    pub damping: f32,
    pub mass: f32,
    /// `true` for duration/bounce-resolved springs (`{bounce: 0}`): they run
    /// for the resolved duration and ignore inherited velocity on retarget.
    pub is_time_defined: bool,
}

impl Default for SpringOptions {
    fn default() -> Self {
        Self {
            stiffness: 100.0,
            damping: 10.0,
            mass: 1.0,
            is_time_defined: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum SpringKind {
    Underdamped,
    CriticallyDamped,
}

/// A motion-dom spring: an analytic, retargetable value animation.
///
/// The value follows the closed-form solution of the damped harmonic
/// oscillator, so retargeting mid-flight preserves position and velocity —
/// exactly like motion's `useSpring`.
#[derive(Debug, Clone, Copy)]
pub struct Spring {
    options: SpringOptions,
    kind: SpringKind,
    target: f32,
    /// Solution coefficients captured at `start_time` (seconds).
    delta0: f32,
    a: f32,
    omega: f32,
    decay: f32,
    w: f32,
    start_time: f64,
    /// Rest thresholds (motion-dom picks them from the retarget delta scale:
    /// |delta| < 5 → 0.01 / 0.005, else 2 / 0.5; velocity is compared in
    /// px/ms, distance in px).
    rest_speed: f32,
    rest_delta: f32,
}

impl Spring {
    /// A spring with motion-dom's physics defaults (what `useSpring(value)`
    /// without options produces).
    pub fn new_physics(current: f32, target: f32, now: f64) -> Self {
        Self::with_options(current, target, now, SpringOptions::default())
    }

    pub fn with_options(current: f32, target: f32, now: f64, options: SpringOptions) -> Self {
        let mut spring = Self {
            options,
            kind: SpringKind::Underdamped,
            target,
            delta0: 0.0,
            a: 0.0,
            omega: 0.0,
            decay: 0.0,
            w: 0.0,
            start_time: now,
            rest_speed: 0.0,
            rest_delta: 0.0,
        };
        spring.restart(current, now);
        spring
    }

    /// motion-dom's `setRestThresholds`: the defaults depend on the scale of
    /// the (re)target delta.
    fn set_rest_thresholds(&mut self, delta: f32) {
        let granular = delta.abs() < 5.0;
        self.rest_speed = if granular { 0.01 } else { 2.0 };
        self.rest_delta = if granular { 0.005 } else { 0.5 };
    }

    fn restart(&mut self, current: f32, now: f64) {
        let SpringOptions {
            stiffness,
            damping,
            mass,
            ..
        } = self.options;
        let zeta = damping / (2.0 * (stiffness * mass).sqrt());
        let omega0 = (stiffness / mass).sqrt();
        self.decay = zeta * omega0;
        self.delta0 = current - self.target;
        self.set_rest_thresholds(self.delta0);
        self.start_time = now;
        if zeta < 1.0 {
            self.kind = SpringKind::Underdamped;
            self.omega = omega0 * (1.0 - zeta * zeta).sqrt();
            // u(t) = e^(-decay t) * (delta0 * cos(ωt) + A * sin(ωt)),
            // u = x - target; A chosen so u'(0) = v0 = 0 at start.
            self.a = self.decay * self.delta0 / self.omega;
            self.w = 0.0;
        } else {
            self.kind = SpringKind::CriticallyDamped;
            self.w = omega0;
            self.omega = 0.0;
            self.a = 0.0;
        }
    }

    /// Retarget (motion's `spring.set(value)`). Physics springs keep the
    /// current position and velocity; time-defined springs drop the velocity.
    pub fn set(&mut self, target: f32, now: f64) {
        if (target - self.target).abs() < f32::EPSILON {
            return;
        }
        let current = self.current(now);
        let velocity = if self.options.is_time_defined {
            0.0
        } else {
            self.velocity(now)
        };
        self.target = target;
        // Re-anchor the solution at the current state.
        let SpringOptions {
            stiffness,
            damping,
            mass,
            ..
        } = self.options;
        let zeta = damping / (2.0 * (stiffness * mass).sqrt());
        let omega0 = (stiffness / mass).sqrt();
        self.decay = zeta * omega0;
        self.delta0 = current - target;
        self.set_rest_thresholds(self.delta0);
        self.start_time = now;
        if zeta < 1.0 {
            self.kind = SpringKind::Underdamped;
            self.omega = omega0 * (1.0 - zeta * zeta).sqrt();
            self.a = (velocity + self.decay * self.delta0) / self.omega;
            self.w = 0.0;
        } else {
            self.kind = SpringKind::CriticallyDamped;
            self.w = omega0;
            self.omega = 0.0;
            // u(t) = e^(-w t) * (delta0 + (v0 + w * delta0) * t)
            self.a = velocity + self.w * self.delta0;
        }
    }

    /// Snap to `target` immediately (no animation): the spring is re-anchored
    /// at rest on the target. This is what `useSpring(value)` does on mount —
    /// the value is shown from the first paint, not animated toward.
    pub fn jump(&mut self, target: f32, now: f64) {
        self.target = target;
        self.restart(target, now);
    }

    /// The raw solution value at `now` (seconds).
    fn current(&self, now: f64) -> f32 {
        let t = (now - self.start_time).max(0.0) as f32;
        let u = match self.kind {
            SpringKind::Underdamped => {
                let e = (-self.decay * t).exp();
                e * (self.delta0 * (self.omega * t).cos() + self.a * (self.omega * t).sin())
            }
            SpringKind::CriticallyDamped => {
                let e = (-self.w * t).exp();
                e * (self.delta0 + self.a * t)
            }
        };
        self.target + u
    }

    /// The interpolated value at `now` (seconds), snapped to the target once
    /// at rest (motion's `state.value = state.done ? target : current`).
    pub fn value(&self, now: f64) -> f32 {
        if self.is_at_rest(now) {
            self.target
        } else {
            self.current(now)
        }
    }

    /// The velocity (units/second) at `now`.
    pub fn velocity(&self, now: f64) -> f32 {
        let t = (now - self.start_time).max(0.0) as f32;
        match self.kind {
            SpringKind::Underdamped => {
                let e = (-self.decay * t).exp();
                let (s, c) = ((self.omega * t).sin(), (self.omega * t).cos());
                e * ((self.a * self.omega - self.decay * self.delta0) * c
                    - (self.delta0 * self.omega + self.decay * self.a) * s)
            }
            SpringKind::CriticallyDamped => {
                let e = (-self.w * t).exp();
                e * (self.a - self.w * (self.delta0 + self.a * t))
            }
        }
    }

    /// motion-dom's rest check: |velocity| (px/ms) ≤ restSpeed and
    /// |target - current| (px) ≤ restDelta.
    pub fn is_at_rest(&self, now: f64) -> bool {
        let v = self.velocity(now).abs() / 1000.0;
        let d = (self.target - self.current(now)).abs();
        v <= self.rest_speed && d <= self.rest_delta
    }

    pub fn target(&self) -> f32 {
        self.target
    }
}

// ---------------------------------------------------------------------------
// TextEffect: per-word fade-in-blur reveal (tween)
// ---------------------------------------------------------------------------

/// The state of one revealed segment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SegmentState {
    /// 0..1 opacity.
    pub opacity: f32,
    /// Vertical offset in px (20 → 0).
    pub y_offset: f32,
    /// Blur radius in px (12 → 0).
    pub blur: f32,
}

/// The `TextEffect` reveal: `per="word"`, `preset="fade-in-blur"`.
///
/// Segments are split on `/(\s+)/` (whitespace runs become their own
/// segments, rendered `whitespace-pre`), each an inline-block span animated
/// with a 0.3s tween staggered 0.05s after the previous one:
/// opacity 0→1, translateY 20→0, blur 12px→0. The tween ease is motion's
/// default `easeOut` (the item variant's transition is `{duration: 0.3}`
/// with no ease). The container fades opacity 0→1 over the same duration
/// with motion-dom's default non-transform ease `[0.25, 0.1, 0.35, 1]`.
/// Plays once, on mount.
pub struct TextEffect {
    segments: Vec<String>,
    stagger: f32,
    duration: f32,
    container_ease: CubicBezier,
    item_ease: CubicBezier,
    start_time: Option<f64>,
}

impl TextEffect {
    /// Split `text` like the reference: `text.split(/(\s+)/)`.
    pub fn new(text: &str) -> Self {
        let mut segments = Vec::new();
        let mut rest = text;
        while !rest.is_empty() {
            match rest.find(char::is_whitespace) {
                Some(0) => {
                    // Leading whitespace run.
                    let end = rest.find(|c: char| !c.is_whitespace()).unwrap_or(rest.len());
                    segments.push(rest[..end].to_string());
                    rest = &rest[end..];
                }
                Some(i) => {
                    segments.push(rest[..i].to_string());
                    let end = rest[i..]
                        .find(|c: char| !c.is_whitespace())
                        .map(|j| i + j)
                        .unwrap_or(rest.len());
                    segments.push(rest[i..end].to_string());
                    rest = &rest[end..];
                }
                None => {
                    segments.push(rest.to_string());
                    rest = "";
                }
            }
        }
        Self {
            segments,
            stagger: 0.05,
            duration: 0.3,
            container_ease: CubicBezier::MOTION_DEFAULT,
            item_ease: CubicBezier::EASE_OUT,
            start_time: None,
        }
    }

    pub fn segments(&self) -> &[String] {
        &self.segments
    }

    /// Begin the reveal (mount).
    pub fn start(&mut self, now: f64) {
        if self.start_time.is_none() {
            self.start_time = Some(now);
        }
    }

    fn elapsed(&self, now: f64) -> Option<f32> {
        self.start_time.map(|t0| (now - t0).max(0.0) as f32)
    }

    /// The container's opacity at `now`.
    pub fn container_opacity(&self, now: f64) -> f32 {
        match self.elapsed(now) {
            Some(e) => self.container_ease.eval((e / self.duration).clamp(0.0, 1.0)),
            None => 0.0,
        }
    }

    /// The state of segment `index` at `now`, or `None` before it starts.
    pub fn segment_state(&self, index: usize, now: f64) -> Option<SegmentState> {
        let e = self.elapsed(now)?;
        let local = (e - index as f32 * self.stagger) / self.duration;
        if local <= 0.0 {
            return Some(SegmentState {
                opacity: 0.0,
                y_offset: 20.0,
                blur: 12.0,
            });
        }
        let t = self.item_ease.eval(local.clamp(0.0, 1.0));
        Some(SegmentState {
            opacity: t,
            y_offset: 20.0 * (1.0 - t),
            blur: 12.0 * (1.0 - t),
        })
    }

    /// True once every segment has fully revealed.
    pub fn is_done(&self, now: f64) -> bool {
        match self.elapsed(now) {
            Some(e) => e >= self.duration + (self.segments.len().saturating_sub(1)) as f32 * self.stagger,
            None => false,
        }
    }

    pub fn started(&self) -> bool {
        self.start_time.is_some()
    }
}

// ---------------------------------------------------------------------------
// AnimatedNumber: spring-interpolated, locale-formatted count
// ---------------------------------------------------------------------------

/// The pending-count badge: `useSpring(queue.length)` rendered as
/// `Math.round(spring).toLocaleString()`.
pub struct AnimatedNumber {
    spring: Spring,
    target: i64,
}

impl AnimatedNumber {
    pub fn new(initial: i64, now: f64) -> Self {
        Self {
            spring: Spring::new_physics(initial as f32, initial as f32, now),
            target: initial,
        }
    }

    pub fn set(&mut self, value: i64, now: f64) {
        if value != self.target {
            self.target = value;
            self.spring.set(value as f32, now);
        }
    }

    /// Snap the displayed count to `value` instantly — the reference mounts
    /// the badge (`queue.length > 0 && <Tag>…</Tag>`) with `useSpring(value)`,
    /// so the first paint already shows the current count.
    pub fn jump(&mut self, value: i64, now: f64) {
        self.target = value;
        self.spring.jump(value as f32, now);
    }

    /// `Math.round(spring).toLocaleString()` — JS `Math.round` rounds half
    /// toward +∞; `toLocaleString()` groups thousands with commas (the
    /// reference runs in an en-US locale).
    pub fn display(&self, now: f64) -> String {
        let v = self.spring.value(now);
        let rounded = (v + 0.5).floor();
        let rounded = if rounded.is_finite() { rounded as i64 } else { self.target };
        format_grouped(rounded)
    }

    pub fn is_settled(&self, now: f64) -> bool {
        self.spring.is_at_rest(now)
    }

    pub fn target(&self) -> i64 {
        self.target
    }

    /// The display with the spring jumped straight to its target (what
    /// motion renders under `prefers-reduced-motion`).
    pub fn display_reduced(&self) -> String {
        format_grouped(self.target)
    }
}

/// `n.toLocaleString("en-US")`.
fn format_grouped(n: i64) -> String {
    if n < 0 {
        return format!("-{}", format_grouped(-n));
    }
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cubic_bezier_endpoints_and_midpoint() {
        for e in [CubicBezier::EASE_OUT, CubicBezier::MOTION_DEFAULT] {
            assert_eq!(e.eval(0.0), 0.0);
            assert_eq!(e.eval(1.0), 1.0);
            assert!(e.eval(0.5) > 0.0 && e.eval(0.5) < 1.0);
        }
        // easeOut is front-loaded: past half at the midpoint.
        assert!(CubicBezier::EASE_OUT.eval(0.5) > 0.5);
        // motion-dom's default non-transform ease [0.25, 0.1, 0.35, 1] is a
        // shallower ease-out (front-loaded): eval(0.5) ≈ 0.7544, not 0.5.
        assert!((CubicBezier::MOTION_DEFAULT.eval(0.5) - 0.7544).abs() < 1e-3);
    }

    #[test]
    fn text_effect_splits_like_js_split_whitespace_capture() {
        let te = TextEffect::new("Slack Approval Gateway");
        assert_eq!(
            te.segments(),
            &["Slack", " ", "Approval", " ", "Gateway"]
        );
        let te2 = TextEffect::new("  a  b ");
        assert_eq!(te2.segments(), &["  ", "a", "  ", "b", " "]);
        // The app title is a single word: one segment, no stagger.
        let te3 = TextEffect::new("SpeakEasy");
        assert_eq!(te3.segments(), &["SpeakEasy"]);
    }

    #[test]
    fn text_effect_reveals_in_order_and_settles() {
        let mut te = TextEffect::new("A B C");
        te.start(1.0);
        // Before the first segment's stagger: everything at the start state.
        let s0 = te.segment_state(0, 1.0).unwrap();
        assert_eq!(s0.opacity, 0.0);
        assert_eq!(s0.y_offset, 20.0);
        assert_eq!(s0.blur, 12.0);
        // Segment 0 finishes at 1.0 + 0.3.
        let s0 = te.segment_state(0, 1.3).unwrap();
        assert!((s0.opacity - 1.0).abs() < 1e-3);
        assert!(s0.y_offset.abs() < 1e-3);
        assert!(s0.blur.abs() < 1e-3);
        // Segment 1 starts at 1.05 (stagger 0.05).
        let s1 = te.segment_state(1, 1.05).unwrap();
        assert!(s1.opacity < 0.05, "{}", s1.opacity);
        let s1 = te.segment_state(1, 1.35).unwrap();
        assert!((s1.opacity - 1.0).abs() < 1e-3);
        // Done after the last segment: 0.3 + 4*0.05 = 0.5s.
        assert!(te.is_done(1.5));
        assert!(!te.is_done(1.4));
        // Container opacity follows its own 0.3s tween.
        assert!((te.container_opacity(1.3) - 1.0).abs() < 1e-3);
        assert_eq!(te.container_opacity(0.5), 0.0);
    }

    #[test]
    fn spring_physics_defaults_match_motion() {
        // stiffness=100, damping=10, mass=1 → ζ=0.5, ω₀=10, decay=5,
        // ωd = ω₀√(1-ζ²) = 8.660254.
        let s = Spring::new_physics(0.0, 1.0, 0.0);
        assert!((s.value(0.0) - 0.0).abs() < 1e-6);
        // At t=0.1: u = e^(-0.5) * (delta0 * cos(ωd·t) + A * sin(ωd·t)),
        // delta0 = from - to = -1, A = decay * delta0 / ωd = -5/8.660254.
        let w = 8.660254f32;
        let expected = 1.0 + (-0.5f32).exp() * (-1.0 * (w * 0.1f32).cos()
            + (-5.0 / w) * (w * 0.1f32).sin());
        assert!((s.value(0.1) - expected).abs() < 1e-5, "{}", s.value(0.1));
        // It overshoots (underdamped) then settles.
        let peak = s.value(0.4);
        assert!(peak > 1.0, "peak {peak}");
        assert!(s.is_at_rest(3.0));
    }

    #[test]
    fn spring_retarget_preserves_position_and_velocity() {
        let mut s = Spring::new_physics(0.0, 10.0, 0.0);
        let v_at = s.value(0.2);
        let vel_at = s.velocity(0.2);
        s.set(20.0, 0.2);
        assert!((s.value(0.2) - v_at).abs() < 1e-4, "position jump on retarget");
        assert!((s.velocity(0.2) - vel_at).abs() < 1e-3, "velocity jump on retarget");
        assert!((s.target() - 20.0).abs() < 1e-6);
    }


    #[test]
    fn animated_number_rounds_and_groups_like_js() {
        let mut n = AnimatedNumber::new(0, 0.0);
        assert_eq!(n.display(0.0), "0");
        n.set(3, 0.0);
        // Mid-animation the display is the rounded interpolated value.
        let mid = n.display(0.05);
        assert!(mid.parse::<i64>().unwrap() >= 0);
        // Settled at the target.
        let settled = n.display(5.0);
        assert_eq!(settled, "3");
        // Grouping.
        let mut big = AnimatedNumber::new(1234567, 0.0);
        assert_eq!(big.display(5.0), "1,234,567");
        big.set(999, 0.0);
        assert_eq!(big.display(5.0), "999");
    }


}
