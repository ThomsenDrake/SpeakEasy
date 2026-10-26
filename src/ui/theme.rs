//! Design tokens for the recreated Once UI theme.
//!
//! The values are the *resolved* (oklch→sRGB) custom properties of the
//! reference UI with `data-theme=light|dark`, `data-neutral=gray`,
//! `data-brand=blue`, `data-accent=indigo`, `data-solid=contrast`,
//! `data-border=rounded`, `data-surface=filled` (see `ui/src/main.tsx` in
//! the pre-migration revision), captured from the running reference app.
//! Spacing/radius values are the Once UI `--static-space-*` /
//! `--static-radius-*` tokens (1rem = 15px at the reference's root font size).

use egui::{Color32, CornerRadius};

use crate::ui::text::{
    TextSpec, BODY_DEFAULT_S, BODY_DEFAULT_XS, BODY_STRONG_M, BODY_STRONG_S, BUTTON_LABEL,
    CODE_DEFAULT_S, CODE_DEFAULT_XS, FIELD_TEXT, LABEL_DEFAULT_M, LABEL_DEFAULT_S,
    LABEL_DEFAULT_XS,
};

/// Which appearance the popup is rendered in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Light,
    Dark,
}

impl Theme {
    pub fn is_dark(self) -> bool {
        matches!(self, Theme::Dark)
    }
}

/// Per-frame resolved metrics: Once UI's rem scale.
///
/// The reference's root font size is *responsive* (Once UI's
/// `--font-scaling-*` behind `html { font-size: … }` media queries):
/// 15px at viewport widths ≤768px (`--font-scaling-mobile`), 16px above
/// (`--font-scaling-tablet`/`--font-scaling-desktop`). Every rem-based token
/// — spacing, radii, component paddings, body/label/code type — scales with
/// it; absolute values (1px borders, `--static-space-1: 1px`, literal icon
/// sizes, the Spotlight's 260px `size` prop) do not.
///
/// The display sizes refine once more at ≤768px (`--font-size-display-*`):
/// `font-m` is 2.25rem/2.5rem there, 2.75rem/3rem at ≤1024px, 3rem/3.25rem
/// above; `font-s` is 2rem/2.5rem, 2.25rem/2.5rem, 2.5rem/3rem.
///
/// All values here are resolved from the *current* window width, so a
/// resized window re-lays out exactly like the reference.
#[derive(Clone, Copy)]
pub struct Metrics {
    /// The root font size in px (15 or 16).
    pub root: f32,
    /// `root / 15`: multiply any captured root-15 constant by this.
    pub s: f32,
    /// The window width (CSS px) these metrics were resolved for.
    pub width: f32,
    /// `font-display font-strong font-m` — the header title.
    pub title: TextSpec,
    /// `display-strong-s` — the request-card heading.
    pub heading: TextSpec,
    /// `body-default-s`.
    pub body_s: TextSpec,
    /// `body-default-xs`.
    pub body_xs: TextSpec,
    /// `body-strong-s`.
    pub body_strong_s: TextSpec,
    /// `body-strong-m`.
    pub body_strong_m: TextSpec,
    /// `label-default-xs` — section labels.
    pub label_xs: TextSpec,
    /// `label-default-s` — tag/badge labels.
    pub label_s: TextSpec,
    /// `label-default-m` — the textarea label.
    pub label_m: TextSpec,
    /// `font-label font-s font-strong` — button labels.
    pub button: TextSpec,
    /// `code-default-s` — the proposed message.
    pub code_s: TextSpec,
    /// `code-default-xs` — the conversation link.
    pub code_xs: TextSpec,
    /// The textarea value text (`body-default`, root size).
    pub field: TextSpec,
    /// `--static-space-1` is absolute (`1px`); every other space token is
    /// rem-based and scales.
    pub sp1: f32,
    pub sp2: f32,
    pub sp4: f32,
    pub sp8: f32,
    pub sp12: f32,
    pub sp16: f32,
    pub sp20: f32,
    pub sp24: f32,
    pub sp32: f32,
    pub sp40: f32,
    pub sp48: f32,
    pub sp56: f32,
    /// `--radius-xs` / `--radius-s` / `--radius-m` / `--radius-l`
    /// (`data-border=rounded`: 1rem / 1.25rem / 1.25rem / 1.75rem).
    pub radius_xs: f32,
    pub radius_s: f32,
    pub radius_m: f32,
    pub radius_l: f32,
    /// `--responsive-width-l` (`maxWidth="l"`): 64rem at ≤1024px,
    /// 75rem above.
    pub card_max_w: f32,
    /// The Once UI Icon size xs (Tag/Button prefix icons): `--static-space-16`.
    pub icon_xs: f32,
    /// The Once UI Icon size l (the empty-state inbox): `--static-space-32`.
    pub icon_l: f32,
    /// The scrollbar width: `--webkit-scrollbar { width: var(--static-space-4) }`.
    pub scrollbar_w: f32,
}

impl Metrics {
    /// Resolve the metrics for a window of `width` CSS px.
    pub fn new(width: f32) -> Self {
        let root = if width <= 768.0 { 15.0 } else { 16.0 };
        let s = root / 15.0;
        let (ts, tl) = if width <= 768.0 {
            (2.25, 2.5)
        } else if width <= 1024.0 {
            (2.75, 3.0)
        } else {
            (3.0, 3.25)
        };
        let (hs, hl) = if width <= 768.0 {
            (2.0, 2.5)
        } else if width <= 1024.0 {
            (2.25, 2.5)
        } else {
            (2.5, 3.0)
        };
        Self {
            root,
            s,
            width,
            title: TextSpec::new(ts * root, 600.0, tl * root, -0.03 * ts * root),
            heading: TextSpec::new(hs * root, 600.0, hl * root, -0.02 * hs * root),
            body_s: BODY_DEFAULT_S.scaled(s),
            body_xs: BODY_DEFAULT_XS.scaled(s),
            body_strong_s: BODY_STRONG_S.scaled(s),
            body_strong_m: BODY_STRONG_M.scaled(s),
            label_xs: LABEL_DEFAULT_XS.scaled(s),
            label_s: LABEL_DEFAULT_S.scaled(s),
            label_m: LABEL_DEFAULT_M.scaled(s),
            button: BUTTON_LABEL.scaled(s),
            code_s: CODE_DEFAULT_S.scaled(s),
            code_xs: CODE_DEFAULT_XS.scaled(s),
            field: FIELD_TEXT.scaled(s),
            sp1: space::S1,
            sp2: space::S2 * s,
            sp4: space::S4 * s,
            sp8: space::S8 * s,
            sp12: space::S12 * s,
            sp16: space::S16 * s,
            sp20: space::S20 * s,
            sp24: space::S24 * s,
            sp32: space::S32 * s,
            sp40: space::S40 * s,
            sp48: space::S48 * s,
            sp56: space::S56 * s,
            radius_xs: radius::XS * s,
            radius_s: radius::S * s,
            radius_m: radius::M * s,
            radius_l: radius::L * s,
            card_max_w: if width > 1024.0 { 75.0 * root } else { 64.0 * root },
            icon_xs: space::S16 * s,
            icon_l: space::S32 * s,
            scrollbar_w: space::S4 * s,
        }
    }

    /// Scale a captured root-15 constant (spacing, radius, padding, …).
    pub fn scale(&self, px: f32) -> f32 {
        px * self.s
    }
}

fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(r, g, b, (a * 255.0).round() as u8)
}

/// The palette for one theme.
#[derive(Clone)]
pub struct Palette {
    /// `--page-background`: the window background.
    pub page: Color32,
    /// `--surface-background`: cards and filled panels.
    pub surface: Color32,
    /// `--neutral-on-background-strong`: primary text.
    pub text_strong: Color32,
    /// `--neutral-on-background-medium`.
    pub text_medium: Color32,
    /// `--neutral-on-background-weak`.
    pub text_weak: Color32,
    /// `--neutral-alpha-weak` = `--default-border`: hairline borders.
    pub border_weak: Color32,
    /// `--neutral-alpha-medium`: tag borders.
    pub border_alpha_medium: Color32,
    /// `--neutral-background-medium`: disabled control fill.
    pub neutral_bg_medium: Color32,
    /// Slack icon: `--brand-on-background-strong`.
    pub brand_on_bg_strong: Color32,
    /// Agent tag text: `--brand-on-background-medium`.
    pub brand_on_bg_medium: Color32,
    /// Agent tag fill: `--brand-background-weak`.
    pub brand_bg_weak: Color32,
    /// Agent tag border: `--brand-alpha-medium`.
    pub brand_alpha_medium: Color32,
    /// Primary button fill: `--brand-solid-medium`.
    pub brand_solid_medium: Color32,
    /// Primary button hover fill / textarea focus ring: `--brand-solid-strong`.
    pub brand_solid_strong: Color32,
    /// Primary button label: `--brand-on-solid-strong`.
    pub brand_on_solid_strong: Color32,
    /// Deny button fill: `--danger-solid-medium`.
    pub danger_solid_medium: Color32,
    /// Deny button hover fill: `--danger-solid-strong`.
    pub danger_solid_strong: Color32,
    /// Deny button label: `--danger-on-solid-strong` (white).
    pub danger_on_solid_strong: Color32,
    /// Submission-error text: `--danger-on-background-medium`.
    pub danger_on_bg_medium: Color32,
    /// Pending badge fill: `--warning-background-weak`.
    pub warning_bg_weak: Color32,
    /// Pending badge border: `--warning-alpha-medium`.
    pub warning_alpha_medium: Color32,
    /// Pending badge + cooldown text: `--warning-on-background-medium`.
    pub warning_on_bg_medium: Color32,
    /// Status dot: `--function-warning-700`.
    pub warning_dot: Color32,
    /// Destination row + info tag text: `--info-on-background-medium`
    /// (gray while `data-neutral=gray`).
    pub info_on_bg_medium: Color32,
    /// Info tag fill: `--info-background-weak` (= surface).
    pub info_bg_weak: Color32,
    /// Info tag border: `--info-alpha-medium` (= neutral alpha medium).
    pub info_alpha_medium: Color32,
    /// Neutral tag fill (raw destination / thread tags): `--neutral-background-weak`.
    pub neutral_tag_bg: Color32,
    /// Neutral tag text: `--neutral-on-background-medium`.
    pub neutral_tag_text: Color32,
    /// Neutral tag border: `--neutral-alpha-medium`.
    pub neutral_tag_border: Color32,
    /// Textarea box border (unfocused): `--neutral-border-medium` (dark) /
    /// `--neutral-border-strong` (light override).
    pub field_border: Color32,
    /// Textarea box border (focused): `--neutral-border-strong`.
    pub field_border_focused: Color32,
    /// Textarea box fill: `--neutral-background-weak` (light) /
    /// `--neutral-alpha-weak` (dark).
    pub field_bg: Color32,
    /// Textarea placeholder: `--neutral-on-background-weak`.
    pub placeholder: Color32,
    /// Keyboard focus ring (Chrome `:focus-visible` outline, macOS):
    /// `rgb(0, 95, 204)` light / `rgb(153, 200, 255)` dark.
    pub focus_ring: Color32,
    /// Text selection highlight (macOS system selection, translucent).
    pub selection: Color32,
}

impl Palette {
    pub fn for_theme(theme: Theme) -> Self {
        match theme {
            Theme::Light => Self {
                page: rgb(0xf9, 0xf9, 0xf9),
                surface: rgb(0xff, 0xff, 0xff),
                text_strong: rgb(0x0a, 0x0a, 0x0a),
                text_medium: rgb(0x3f, 0x3f, 0x3f),
                text_weak: rgb(0x59, 0x59, 0x59),
                border_weak: rgba(149, 149, 149, 0.15),
                border_alpha_medium: rgba(149, 149, 149, 0.3),
                neutral_bg_medium: rgb(0xed, 0xed, 0xed),
                brand_on_bg_strong: rgb(0x0a, 0x07, 0x1b),
                brand_on_bg_medium: rgb(0x03, 0x3c, 0xf8),
                brand_bg_weak: rgb(0xff, 0xff, 0xff),
                brand_alpha_medium: rgba(90, 147, 252, 0.3),
                brand_solid_medium: rgb(0x00, 0x00, 0x00),
                brand_solid_strong: rgb(0x3f, 0x3f, 0x3f),
                brand_on_solid_strong: rgb(0xff, 0xff, 0xff),
                danger_solid_medium: rgb(0xb6, 0x02, 0x0c),
                danger_solid_strong: rgb(0xe9, 0x05, 0x07),
                danger_on_solid_strong: rgb(0xff, 0xff, 0xff),
                danger_on_bg_medium: rgb(0xb6, 0x02, 0x0c),
                warning_bg_weak: rgb(0xff, 0xff, 0xff),
                warning_alpha_medium: rgba(224, 123, 0, 0.3),
                warning_on_bg_medium: rgb(0x92, 0x42, 0x05),
                warning_dot: rgb(0xfe, 0x9c, 0x0b),
                info_on_bg_medium: rgb(0x59, 0x59, 0x59),
                info_bg_weak: rgb(0xff, 0xff, 0xff),
                info_alpha_medium: rgba(149, 149, 149, 0.3),
                neutral_tag_bg: rgb(0xff, 0xff, 0xff),
                neutral_tag_text: rgb(0x3f, 0x3f, 0x3f),
                neutral_tag_border: rgba(149, 149, 149, 0.3),
                field_border: rgb(0xd2, 0xd2, 0xd2),
                field_border_focused: rgb(0xd2, 0xd2, 0xd2),
                field_bg: rgb(0xff, 0xff, 0xff),
                placeholder: rgb(0x59, 0x59, 0x59),
                focus_ring: rgb(0x00, 0x5f, 0xcc),
                selection: rgba(0, 95, 204, 0.30),
            },
            Theme::Dark => Self {
                page: rgb(0x0a, 0x0a, 0x0a),
                surface: rgb(0x15, 0x15, 0x15),
                text_strong: rgb(0xff, 0xff, 0xff),
                text_medium: rgb(0xe0, 0xe0, 0xe0),
                text_weak: rgb(0xb2, 0xb2, 0xb2),
                border_weak: rgba(149, 149, 149, 0.15),
                border_alpha_medium: rgba(149, 149, 149, 0.3),
                neutral_bg_medium: rgb(0x15, 0x15, 0x15),
                brand_on_bg_strong: rgb(0xff, 0xff, 0xff),
                brand_on_bg_medium: rgb(0xcb, 0xe3, 0xfb),
                brand_bg_weak: rgb(0x0a, 0x07, 0x1b),
                brand_alpha_medium: rgba(90, 147, 252, 0.3),
                brand_solid_medium: rgb(0xff, 0xff, 0xff),
                brand_solid_strong: rgb(0xed, 0xed, 0xed),
                brand_on_solid_strong: rgb(0x00, 0x00, 0x00),
                danger_solid_medium: rgb(0xb6, 0x02, 0x0c),
                danger_solid_strong: rgb(0xe9, 0x05, 0x07),
                danger_on_solid_strong: rgb(0xff, 0xff, 0xff),
                danger_on_bg_medium: rgb(0xfd, 0xd8, 0xd2),
                warning_bg_weak: rgb(0x10, 0x07, 0x04),
                warning_alpha_medium: rgba(224, 123, 0, 0.3),
                warning_on_bg_medium: rgb(0xfe, 0xde, 0x79),
                warning_dot: rgb(0xfe, 0x9c, 0x0b),
                info_on_bg_medium: rgb(0xe0, 0xe0, 0xe0),
                info_bg_weak: rgb(0x0a, 0x0a, 0x0a),
                info_alpha_medium: rgba(149, 149, 149, 0.3),
                neutral_tag_bg: rgb(0x0a, 0x0a, 0x0a),
                neutral_tag_text: rgb(0xe0, 0xe0, 0xe0),
                neutral_tag_border: rgba(149, 149, 149, 0.3),
                field_border: rgb(0x3f, 0x3f, 0x3f),
                field_border_focused: rgb(0x59, 0x59, 0x59),
                field_bg: rgba(149, 149, 149, 0.15),
                placeholder: rgb(0xb2, 0xb2, 0xb2),
                focus_ring: rgb(0x99, 0xc8, 0xff),
                selection: rgba(153, 200, 255, 0.30),
            },
        }
    }
}

/// Once UI spacing tokens (`--static-space-*`), in px. The reference root
/// font size is 15px (1rem = 15px); the scale is 0.9375px per step, except
/// `--static-space-1` which is exactly 1px.
pub mod space {
    pub const S1: f32 = 1.0;
    pub const S2: f32 = 1.875;
    pub const S4: f32 = 3.75;
    pub const S8: f32 = 7.5;
    pub const S12: f32 = 11.25;
    pub const S16: f32 = 15.0;
    pub const S20: f32 = 18.75;
    pub const S24: f32 = 22.5;
    pub const S32: f32 = 30.0;
    pub const S40: f32 = 37.5;
    pub const S48: f32 = 45.0;
    pub const S56: f32 = 52.5;
}

/// Once UI radius tokens (`--static-radius-*`), in px.
pub mod radius {
    pub const XS: f32 = 15.0;
    pub const S: f32 = 18.75;
    pub const M: f32 = 18.75;
    pub const L: f32 = 26.25;
    pub const XL: f32 = 30.0;
}

/// Convert a CSS px radius to egui's per-corner rounding. egui quantizes to
/// whole points (scaled by pixels-per-point when tessellating), so fractional
/// radii (18.75, 26.25) round to the nearest point — a sub-pixel difference.
pub fn rounding(px: f32) -> CornerRadius {
    CornerRadius::same(px.max(0.0).round() as u8)
}

/// The transition the reference applies to interactive controls
/// (`transition: all 0.2s ease-in-out`, `--transition-micro-medium`).
pub const CONTROL_TRANSITION: std::time::Duration = std::time::Duration::from_millis(200);

/// `outline: 1px solid var(--brand-solid-strong); outline-offset: 2px` — the
/// textarea focus ring (`focusRing` prop; the reference's computed outline is
/// `oklch(0.9461 0 0) solid 1px`, offset 2px — measured 1px band at
/// [649, 650) at both dsf 1 and dsf 2).
pub const FIELD_FOCUS_RING_WIDTH: f32 = 1.0;
pub const FIELD_FOCUS_RING_OFFSET: f32 = 2.0;
