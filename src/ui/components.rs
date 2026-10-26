//! Native recreations of the Once UI components the approval popup uses.
//!
//! Every value here is the *resolved* geometry of the reference UI (Once UI
//! 2.5.0 with `data-neutral=gray data-brand=blue data-accent=indigo
//! data-solid=contrast data-border=rounded data-surface=filled`), captured
//! from the running reference app at its default root font size (15px).
//! Once UI is rem-based and its root font size is responsive (15px at
//! viewport widths ≤768px, 16px above — see [`crate::ui::theme::Metrics`]),
//! so every rem-based value here is resolved per frame through `Ctx::m`;
//! absolute values (1px borders, `--static-space-1`, literal icon sizes)
//! are not scaled. Chrome snaps backgrounds and 1px borders to whole device
//! pixels (round-half-up), so the painting helpers snap rects before
//! filling/stroking; the stroke-based borders and focus rings land on exact
//! device pixels, matching the reference's rendered output. Text itself is
//! painted at its exact subpixel layout position, like Chrome.

use std::sync::Arc;

use epaint::{text::Galley, ColorMode, PathShape, PathStroke};
use egui::{
    pos2, vec2, Color32, Context, Frame, Id, Key, Label, Painter, Pos2, Rect, Response, Role,
    Sense, Shape, StrokeKind, Ui, Vec2, WidgetInfo, WidgetText,
};

use crate::ui::anim::CubicBezier;
use crate::ui::icons::{paint_icon, Icon};
use crate::ui::text::TextSpec;
use crate::ui::theme::{Palette, Metrics, FIELD_FOCUS_RING_OFFSET, FIELD_FOCUS_RING_WIDTH};

// ---------------------------------------------------------------------------
// Pixel snapping (Chrome's border-box snapping)
// ---------------------------------------------------------------------------

/// Round an edge to a whole CSS pixel, half away from zero. Chrome snaps
/// painted border boxes (and backgrounds) to whole *CSS* pixels per edge —
/// not to device pixels — at every device scale factor: measured against the
/// reference at dsf 2, the card ([22.5, 91, 675, 557.375) -> [23, 698) x
/// [91, 648)), the textarea ([594.875, 647.375) -> [595, 647)), the agent tag
/// ([114.5, 135.25) -> [115, 135)), the message panel ([210.125, 264.625) ->
/// [210, 265)), the conversation panel ([442.875, 553.625) -> [443, 554)) and
/// the buttons ([594.875, 624.875) -> [595, 625)) all render on the integer
/// CSS-pixel grid at dsf 1 and dsf 2 alike. `ppp` is kept for call-site
/// compatibility; the snapping itself is scale-independent.
pub fn snap_edge(x: f32, _ppp: f32) -> f32 {
    x.round()
}

/// Snap a rect's edges to whole CSS pixels.
pub fn snap_rect(rect: Rect, ppp: f32) -> Rect {
    Rect::from_min_max(
        pos2(snap_edge(rect.left(), ppp), snap_edge(rect.top(), ppp)),
        pos2(snap_edge(rect.right(), ppp), snap_edge(rect.bottom(), ppp)),
    )
}

// ---------------------------------------------------------------------------
// Rounded-rect paths (exact f32 radii; epaint's CornerRadius quantizes to u8)
// ---------------------------------------------------------------------------

/// A clockwise rounded-rect outline as a polyline with arc samples.
pub fn rounded_rect_path(rect: Rect, radius: f32) -> Vec<Pos2> {
    let r = radius
        .max(0.0)
        .min(rect.width() / 2.0)
        .min(rect.height() / 2.0);
    if r <= 0.0 {
        return vec![
            rect.left_top(),
            rect.right_top(),
            rect.right_bottom(),
            rect.left_bottom(),
        ];
    }
    // ~1.5px per arc segment keeps the tessellated curve smooth.
    let steps = ((r * std::f32::consts::FRAC_PI_2 / 1.5).ceil() as usize).clamp(6, 96);
    let mut pts = Vec::with_capacity(8 + 4 * (steps + 1));
    pts.push(pos2(rect.left() + r, rect.top()));
    pts.push(pos2(rect.right() - r, rect.top()));
    let corners = [
        (rect.right() - r, rect.top() + r, -std::f32::consts::FRAC_PI_2),
        (rect.right() - r, rect.bottom() - r, 0.0),
        (rect.left() + r, rect.bottom() - r, std::f32::consts::FRAC_PI_2),
        (rect.left() + r, rect.top() + r, std::f32::consts::PI),
    ];
    for (cx, cy, a0) in corners {
        for i in 0..=steps {
            let a = a0 + (i as f32 / steps as f32) * std::f32::consts::FRAC_PI_2;
            pts.push(pos2(cx + r * a.cos(), cy + r * a.sin()));
        }
    }
    pts
}

/// Fill a rounded rect (anti-aliased by the tessellator).
pub fn fill_rounded(painter: &Painter, rect: Rect, radius: f32, color: Color32) {
    if color == Color32::TRANSPARENT {
        return;
    }
    painter.add(Shape::Path(PathShape {
        points: rounded_rect_path(rect, radius),
        closed: true,
        fill: color,
        stroke: PathStroke::NONE,
    }));
}

/// Fill the snapped border box (Chrome pixel-snaps backgrounds; the fill
/// extends under the semi-transparent border, `background-clip: border-box`).
pub fn fill_box(painter: &Painter, rect: Rect, radius: f32, color: Color32, ppp: f32) {
    fill_rounded(painter, snap_rect(rect, ppp), radius, color);
}

/// A Chrome-style 1px border: the band between the snapped border box and
/// the snapped box inset by 1pt, stroked as a device-aligned centerline.
pub fn stroke_border(painter: &Painter, rect: Rect, radius: f32, color: Color32, ppp: f32) {
    if color == Color32::TRANSPARENT {
        return;
    }
    let outer = snap_rect(rect, ppp);
    let inner = snap_rect(rect.shrink(1.0), ppp);
    let cl = Rect::from_min_max(
        pos2(
            (outer.left() + inner.left()) / 2.0,
            (outer.top() + inner.top()) / 2.0,
        ),
        pos2(
            (outer.right() + inner.right()) / 2.0,
            (outer.bottom() + inner.bottom()) / 2.0,
        ),
    );
    let path = rounded_rect_path(cl, (radius - 0.5).max(0.0));
    painter.add(Shape::Path(PathShape {
        points: path,
        closed: true,
        fill: Color32::TRANSPARENT,
        stroke: PathStroke {
            width: 1.0,
            color: ColorMode::Solid(color),
            kind: StrokeKind::Middle,
        },
    }));
}

/// A Chrome-style 1px top border (a full-width band at the rect's top edge).
pub fn stroke_border_top(painter: &Painter, rect: Rect, color: Color32, ppp: f32) {
    stroke_hline(painter, rect.left(), rect.right(), rect.top(), color, ppp);
}

/// A Chrome-style 1px bottom border (a full-width band at the rect's bottom
/// edge, inside the box).
pub fn stroke_border_bottom(painter: &Painter, rect: Rect, color: Color32, ppp: f32) {
    stroke_hline(
        painter,
        rect.left(),
        rect.right(),
        rect.bottom() - 1.0,
        color,
        ppp,
    );
}

/// Stroke a 1px horizontal band at layout `[y, y+1)` (Chrome snaps both
/// edges round-half-up to device pixels).
fn stroke_hline(painter: &Painter, x0: f32, x1: f32, y: f32, color: Color32, ppp: f32) {
    if color == Color32::TRANSPARENT {
        return;
    }
    let a = snap_edge(y, ppp);
    let b = snap_edge(y + 1.0, ppp);
    let cl = (a + b) / 2.0;
    painter.add(Shape::line_segment(
        [pos2(x0, cl), pos2(x1, cl)],
        egui::Stroke::new(1.0, color),
    ));
}

/// The keyboard focus ring (Chrome's macOS `outline: auto`): a 2px
/// device-snapped band just inside the widget's border box.
pub fn stroke_focus_ring(painter: &Painter, rect: Rect, radius: f32, color: Color32, ppp: f32) {
    if color == Color32::TRANSPARENT {
        return;
    }
    let k_dev = (2.0 * ppp).round().max(1.0);
    let outer = snap_rect(rect, ppp);
    let inset = k_dev / 2.0 / ppp;
    let cl = outer.shrink(inset);
    let path = rounded_rect_path(cl, (radius - inset).max(0.0));
    painter.add(Shape::Path(PathShape {
        points: path,
        closed: true,
        fill: Color32::TRANSPARENT,
        stroke: PathStroke {
            width: k_dev / ppp,
            color: ColorMode::Solid(color),
            kind: StrokeKind::Middle,
        },
    }));
}

/// The textarea focus ring (Once UI `focusRing`): `outline: 1px solid
/// var(--brand-solid-strong); outline-offset: 2px` — a 1-CSS-px band outside
/// the snapped border box (measured [649, 650) against the bottom edge at both
/// dsf 1 and dsf 2).
pub fn stroke_field_ring(
    painter: &Painter,
    rect: Rect,
    radius: f32,
    color: Color32,
    ppp: f32,
) {
    if color == Color32::TRANSPARENT {
        return;
    }
    let w_dev = (FIELD_FOCUS_RING_WIDTH * ppp).floor().max(1.0);
    let off_dev = (FIELD_FOCUS_RING_OFFSET * ppp).round();
    let expand = (off_dev + w_dev / 2.0) / ppp;
    let cl = snap_rect(rect, ppp).expand(expand);
    let path = rounded_rect_path(cl, radius + expand);
    painter.add(Shape::Path(PathShape {
        points: path,
        closed: true,
        fill: Color32::TRANSPARENT,
        stroke: PathStroke {
            width: w_dev / ppp,
            color: ColorMode::Solid(color),
            kind: StrokeKind::Middle,
        },
    }));
}

// ---------------------------------------------------------------------------
// Shared context, focus tracking, events
// ---------------------------------------------------------------------------

/// A user action the card reports back to the app.
#[derive(Debug, Clone, PartialEq)]
pub enum AppEvent {
    Approve,
    Deny,
    SubmitFeedback(String),
    ToggleFeedback,
}

/// Keyboard-focusable controls, in DOM (tab) order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FocusId {
    Textarea,
    SubmitFeedback,
    GiveFeedback,
    Deny,
    Approve,
}

/// Focus tracker implementing the browser's focus model: Tab/Shift+Tab move
/// focus (and show the ring), clicking focuses without the ring, Enter/Space
/// activate.
#[derive(Default)]
pub struct FocusTracker {
    /// Controls in DOM order, rebuilt each frame.
    pub order: Vec<FocusId>,
    pub focused: Option<FocusId>,
    /// True when the focus came from the keyboard (`:focus-visible`).
    pub keyboard: bool,
    /// A button on which Space is currently held.
    pub space_down: Option<FocusId>,
}

impl FocusTracker {
    pub fn register(&mut self, id: FocusId) {
        if !self.order.contains(&id) {
            self.order.push(id);
        }
    }

    pub fn is_focused(&self, id: FocusId) -> bool {
        self.focused == Some(id)
    }

    /// The keyboard focus ring is only drawn for keyboard focus.
    pub fn ring_visible(&self, id: FocusId) -> bool {
        self.focused == Some(id) && self.keyboard
    }

    pub fn focus(&mut self, id: FocusId, keyboard: bool) {
        self.focused = Some(id);
        self.keyboard = keyboard;
    }

    pub fn blur(&mut self) {
        self.focused = None;
        self.keyboard = false;
    }

    /// Move focus to the next/previous control (Tab / Shift+Tab), following
    /// the browser's sequential focus navigation: disabled controls are not
    /// in `order` (they are skipped), and tabbing past either boundary moves
    /// focus to the document body — no control shows a ring — until the next
    /// Tab re-enters at the boundary (measured on the reference: forward
    /// Textarea → Give feedback → Deny → Approve → body → Textarea; backward
    /// Textarea → body → Approve → Deny → Give feedback).
    pub fn move_focus(&mut self, forward: bool) {
        if self.order.is_empty() {
            self.blur();
            return;
        }
        let n = self.order.len();
        let cur = self
            .focused
            .and_then(|f| self.order.iter().position(|&g| g == f));
        let next = match cur {
            None => {
                // The body (or a control that no longer exists) holds focus.
                if forward {
                    Some(self.order[0])
                } else {
                    Some(self.order[n - 1])
                }
            }
            Some(idx) => {
                if forward {
                    if idx + 1 < n {
                        Some(self.order[idx + 1])
                    } else {
                        None // past the last control → the document body
                    }
                } else {
                    if idx > 0 {
                        Some(self.order[idx - 1])
                    } else {
                        None // before the first control → the document body
                    }
                }
            }
        };
        match next {
            Some(id) => self.focus(id, true),
            None => {
                self.focused = None;
                self.keyboard = true;
            }
        }
    }
}

/// Per-frame context handed to the components.
pub struct Ctx<'a> {
    pub ui: &'a mut Ui,
    pub pal: &'a Palette,
    /// Resolved metrics for the current window width (the rem scale).
    pub m: &'a Metrics,
    /// Frame time in seconds (egui's clock).
    pub now: f64,
    pub reduce_motion: bool,
    pub events: &'a mut Vec<AppEvent>,
    pub focus: &'a mut FocusTracker,
    /// Set by interactive controls when they consume the current pointer
    /// press, so the app can blur the textarea when the press lands
    /// elsewhere (browser behavior).
    pub press_consumed: bool,
}

impl<'a> Ctx<'a> {
    pub fn ctx(&self) -> &Context {
        self.ui.ctx()
    }

    pub fn painter(&self) -> &Painter {
        self.ui.painter()
    }

    pub fn ppp(&self) -> f32 {
        self.ui.ctx().pixels_per_point()
    }

    /// Register a focusable control (DOM order) and report whether it is
    /// focused via the keyboard (ring visible).
    fn focusable(&mut self, id: FocusId, response: &Response) -> bool {
        self.focus.register(id);
        if response.clicked_by(egui::PointerButton::Primary) {
            self.focus.focus(id, false);
        }
        // A press that starts on the control is consumed by it (the browser
        // keeps the focus). `is_pointer_button_down_on` is unreliable when
        // the press and release land in the same frame — egui clears the
        // flag while processing the release — so test the press position.
        let pressed_on = self.ctx().input(|i| {
            i.pointer.primary_pressed()
                && i.pointer
                    .latest_pos()
                    .map(|p| response.rect.contains(p))
                    .unwrap_or(false)
        });
        if pressed_on {
            self.press_consumed = true;
        }
        self.focus.ring_visible(id)
    }
}

// ---------------------------------------------------------------------------
// 200ms ease-in-out tweens (Once UI `--transition-micro-medium`)
// ---------------------------------------------------------------------------

const CONTROL_EASE: CubicBezier = CubicBezier::EASE_IN_OUT;

/// A value tweened toward a target over 200ms with the control ease
/// (`transition: all 0.2s ease-in-out`).
#[derive(Clone)]
pub struct Tween {
    value: f32,
    target: f32,
    from: f32,
    start: f64,
}

impl Tween {
    pub fn new(initial: f32) -> Self {
        Self {
            value: initial,
            target: initial,
            from: initial,
            start: 0.0,
        }
    }

    pub fn set(&mut self, target: f32, now: f64) {
        if (target - self.target).abs() > 1e-6 {
            self.from = self.value(now);
            self.target = target;
            self.start = now;
            self.value = self.from;
        }
    }

    pub fn value(&self, now: f64) -> f32 {
        if self.from == self.target {
            return self.target;
        }
        let t = ((now - self.start) / 0.2).clamp(0.0, 1.0);
        let e = CONTROL_EASE.eval(t as f32);
        self.from + (self.target - self.from) * e
    }

    pub fn at_target(&self, now: f64) -> bool {
        (now - self.start) >= 0.2
    }

    /// The tween's target (what it is heading to, or has reached).
    pub fn target_value(&self) -> f32 {
        self.target
    }
}

fn tween_state(ctx: &Context, id: Id, initial: f32) -> Tween {
    ctx.data_mut(|d| {
        d.get_temp::<Tween>(id)
            .unwrap_or_else(|| Tween::new(initial))
    })
}

fn store_tween(ctx: &Context, id: Id, tween: Tween) {
    ctx.data_mut(|d| d.insert_temp(id, tween));
}

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

/// Lay out text in a style. `break_anywhere` means CSS
/// `overflow-wrap: anywhere` (Chrome's line breaking — see
/// [`crate::ui::text::galley_anywhere`]); egui's native `break_anywhere`
/// wraps differently and is only used for the fallback paths.
pub fn galley(
    ctx: &Context,
    spec: TextSpec,
    text: &str,
    color: Color32,
    wrap_width: f32,
    break_anywhere: bool,
) -> Arc<Galley> {
    galley_align(ctx, spec, text, color, wrap_width, break_anywhere, egui::Align::LEFT)
}

/// Lay out text in a style with an explicit horizontal alignment.
pub fn galley_align(
    ctx: &Context,
    spec: TextSpec,
    text: &str,
    color: Color32,
    wrap_width: f32,
    break_anywhere: bool,
    halign: egui::Align,
) -> Arc<Galley> {
    if break_anywhere && wrap_width.is_finite() && halign == egui::Align::LEFT {
        return crate::ui::text::galley_anywhere(ctx, spec, text, color, wrap_width);
    }
    ctx.fonts_mut(|f| f.layout_job(spec.job_align(text, color, wrap_width, break_anywhere, halign)))
}

/// The size of a laid-out galley.
pub fn galley_size(galley: &Galley) -> Vec2 {
    galley.rect.size()
}

/// The CSS block height of a laid-out galley: one line box per row at the
/// style's exact line-height. egui rounds every row to whole physical pixels
/// (and the row cursor with it), so `galley.rect.height()` drifts from the
/// browser's fractional line boxes (37.5 → 38 at 1×, 16.875 → 17 even at
/// 2×). All layout math that advances by a text block must use this.
pub fn block_height(galley: &Galley, spec: TextSpec) -> f32 {
    galley.rows.len() as f32 * spec.line_height
}

/// Paint a galley at its top-left, with every line's baseline placed where
/// the reference (Chrome) renders it (see `text::baseline_correction` and
/// `text::row_baseline_shifts`).
pub fn paint_galley(painter: &Painter, pos: Pos2, galley: &Arc<Galley>) {
    crate::ui::text::paint_galley(painter, pos, galley);
}

/// Measure a single-line text width.
pub fn text_width(ctx: &Context, spec: TextSpec, text: &str, color: Color32) -> f32 {
    // Fit-content boxes in the reference size to Chrome's text advance, and
    // Chrome adds letter-spacing and CoreText's `trak` tracking after the last
    // glyph as well — egui adds letter-spacing only between glyphs. Add the
    // trailing unit back so box widths match the reference.
    galley(ctx, spec, text, color, f32::INFINITY, false).rect.width()
        + spec.letter_spacing
        + spec.trak_tracking()
}

/// Plain (non-interactive) text at `pos`. Returns the galley's rect.
pub fn text_at(
    c: &mut Ctx,
    pos: Pos2,
    spec: TextSpec,
    text: &str,
    color: Color32,
    wrap_width: f32,
    break_anywhere: bool,
) -> Rect {
    let galley = galley(c.ctx(), spec, text, color, wrap_width, break_anywhere);
    let rect = Rect::from_min_size(pos, galley.rect.size());
    let painter = c.painter().with_clip_rect(c.ui.clip_rect());
    paint_galley(&painter, pos, &galley);
    rect
}

/// Selectable text (the reference renders these as ordinary selectable
/// paragraphs). Painted through an interactive `egui::Label`, so egui's
/// native label text selection applies — drag, double/triple-click,
/// shift-extend, keyboard selection and Cmd+C, like the browser. The
/// selection highlight uses `visuals.selection.bg_fill` (set per theme);
/// the selected glyphs keep their own color, as in the reference.
/// Returns the galley's rect.
pub fn selectable_text(
    c: &mut Ctx,
    id: Id,
    pos: Pos2,
    spec: TextSpec,
    text: &str,
    color: Color32,
    wrap_width: f32,
    break_anywhere: bool,
) -> Rect {
    let galley = galley(c.ctx(), spec, text, color, wrap_width, break_anywhere);
    // Place every line's baseline where the reference renders it. When the
    // galley needs no per-row adjustment, shift the whole widget (text,
    // selection highlight and all) by the first-line correction; otherwise
    // hand the label a row-snapped galley and leave the rect in place.
    let snapped = crate::ui::text::snapped_galley(&galley, pos.y);
    let dy = if Arc::ptr_eq(&snapped, &galley) {
        crate::ui::text::baseline_correction(&galley, pos.y)
    } else {
        0.0
    };
    let rect = Rect::from_min_size(pos2(pos.x, pos.y + dy), galley.rect.size());
    let mut child = c
        .ui
        .new_child(egui::UiBuilder::new().id_salt(id).max_rect(rect));
    child.style_mut().visuals.selection.stroke.color = color;
    // Clicking selectable text does NOT consume the press: like any
    // non-focusable element in the browser, it blurs the focused control.
    let _ = child.add(Label::new(WidgetText::Galley(snapped)).selectable(true));
    rect
}

// ---------------------------------------------------------------------------
// Icons
// ---------------------------------------------------------------------------

/// Paint an icon at `size` with top-left `pos`.
pub fn icon_at(c: &Ctx, icon: Icon, pos: Pos2, size: f32, color: Color32) {
    paint_icon(c.painter(), icon, pos, size, color);
}

// ---------------------------------------------------------------------------
// Tag
// ---------------------------------------------------------------------------

/// Tag color scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagScheme {
    Brand,
    Warning,
    Info,
    Neutral,
}

impl TagScheme {
    fn colors(self, pal: &Palette) -> (Color32, Color32, Color32) {
        match self {
            TagScheme::Brand => (
                pal.brand_bg_weak,
                pal.brand_alpha_medium,
                pal.brand_on_bg_medium,
            ),
            TagScheme::Warning => (
                pal.warning_bg_weak,
                pal.warning_alpha_medium,
                pal.warning_on_bg_medium,
            ),
            TagScheme::Info => (pal.info_bg_weak, pal.info_alpha_medium, pal.info_on_bg_medium),
            TagScheme::Neutral => (
                pal.neutral_tag_bg,
                pal.neutral_tag_border,
                pal.neutral_tag_text,
            ),
        }
    }
}

/// The width of a tag's border box (for row layout before painting).
pub fn tag_width(
    ctx: &Context,
    m: &Metrics,
    label: &str,
    prefix_icon: Option<Icon>,
    text_color: Color32,
) -> f32 {
    let label_w = text_width(ctx, m.label_s, label, text_color);
    let icon_w = prefix_icon.map(|_| m.icon_xs + m.sp4).unwrap_or(0.0);
    icon_w + label_w + 2.0 * m.sp8 + 2.0
}

/// A Once UI Tag: rounded, bordered, with an optional prefix icon and a
/// `label-default-s` label. `size_m` selects the size-m padding (space-2
/// vertical); otherwise size-s (space-1, absolute 1px). Returns the tag's
/// border-box rect.
pub fn tag(
    c: &mut Ctx,
    id: Id,
    pos: Pos2,
    scheme: TagScheme,
    size_m: bool,
    label: &str,
    prefix_icon: Option<Icon>,
) -> Rect {
    let (bg, border, text_color) = scheme.colors(c.pal);
    let pad_y = if size_m { c.m.sp2 } else { c.m.sp1 };
    let pad_x = c.m.sp8;
    let content_h = c.m.label_s.line_height; // label line box (icon is also icon_xs)
    let h = content_h + 2.0 * pad_y + 2.0; // + 1px borders

    let w = tag_width(c.ctx(), c.m, label, prefix_icon, text_color);
    let rect = Rect::from_min_size(pos, vec2(w, h));

    let painter = c.painter().with_clip_rect(c.ui.clip_rect());
    fill_box(&painter, rect, c.m.radius_s, bg, c.ppp());
    stroke_border(&painter, rect, c.m.radius_s, border, c.ppp());

    // Content, vertically centered.
    let content_y = rect.top() + 1.0 + pad_y;
    let mut x = rect.left() + 1.0 + pad_x;
    if let Some(icon) = prefix_icon {
        icon_at(c, icon, pos2(x, content_y), c.m.icon_xs, text_color);
        x += c.m.icon_xs + c.m.sp4;
    }
    let galley = galley(c.ctx(), c.m.label_s, label, text_color, f32::INFINITY, false);
    paint_galley(&painter, pos2(x, content_y), &galley);
    let _ = id;
    rect
}

// ---------------------------------------------------------------------------
// Button
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonVariant {
    Primary,
    Secondary,
    Danger,
}

/// A Once UI Button (size s): height space-32 (30px at root 15), padding-x
/// space-8 (7.5), gap space-4 (3.75), radius-s (18.75), icon icon-xs (15)
/// + gap + label (`font-label font-s font-strong`, 12.375/600/15).
/// Returns true when activated (click, or Enter/Space while focused).
pub fn button(
    c: &mut Ctx,
    id: Id,
    pos: Pos2,
    variant: ButtonVariant,
    label: &str,
    icon: Option<Icon>,
    disabled: bool,
    focus_id: FocusId,
) -> bool {
    let pal = c.pal;
    let text_color = match variant {
        ButtonVariant::Primary => pal.brand_on_solid_strong,
        ButtonVariant::Secondary => pal.text_strong,
        ButtonVariant::Danger => pal.danger_on_solid_strong,
    };
    // The label sits in a `px-4` wrapper (padding-x space-4 = 3.75px each
    // side), so its box is the text plus 2×space-4.
    let label_w = text_width(c.ctx(), c.m.button, label, text_color) + 2.0 * c.m.sp4;
    // Once UI Button size s: padding-x space-8, gap space-4, height
    // space-32. Solid variants have a 0px border; the secondary variant
    // always has a 1px border (transparent when disabled), which is part of
    // the button's width.
    let border_w = if variant == ButtonVariant::Secondary { 1.0 } else { 0.0 };
    let content_w = icon.map(|_| c.m.icon_xs + c.m.sp4).unwrap_or(0.0) + label_w;
    let w = content_w + 2.0 * (border_w + c.m.sp8);
    let h = c.m.sp32;
    let rect = Rect::from_min_size(pos, vec2(w, h));

    let sense = if disabled {
        Sense::hover()
    } else {
        Sense::click()
    };
    let response = c.ui.interact(rect, id, sense);
    // Disabled controls are not focusable: the browser skips them in the
    // tab order (and they show no ring).
    let ring = if !disabled {
        c.focusable(focus_id, &response)
    } else {
        false
    };

    // Hover tween (200ms ease-in-out on background/border colors; instant
    // under reduced motion, like the reference's `transition-duration:
    // .01ms` media query).
    let hover_target = if response.hovered() && !disabled { 1.0 } else { 0.0 };
    let mut tween = tween_state(c.ctx(), id, 0.0);
    tween.set(hover_target, c.now);
    let hover = if c.reduce_motion {
        hover_target
    } else {
        tween.value(c.now)
    };
    store_tween(c.ctx(), id, tween);
    let icon_color = if disabled { pal.text_weak } else { text_color };
    let label_color = if disabled { pal.text_weak } else { text_color };

    let (bg, border) = if disabled {
        (pal.neutral_bg_medium, Color32::TRANSPARENT)
    } else {
        match variant {
            ButtonVariant::Primary => (
                lerp_color(pal.brand_solid_medium, pal.brand_solid_strong, hover),
                Color32::TRANSPARENT,
            ),
            ButtonVariant::Secondary => (
                lerp_color(Color32::TRANSPARENT, pal.border_weak, hover),
                lerp_color(pal.border_weak, pal.border_weak, hover),
            ),
            ButtonVariant::Danger => (
                lerp_color(pal.danger_solid_medium, pal.danger_solid_strong, hover),
                Color32::TRANSPARENT,
            ),
        }
    };

    let painter = c.painter().with_clip_rect(c.ui.clip_rect());
    fill_box(&painter, rect, c.m.radius_s, bg, c.ppp());
    if border != Color32::TRANSPARENT {
        stroke_border(&painter, rect, c.m.radius_s, border, c.ppp());
    }

    // Content, centered (the (w - content_w)/2 inset covers the symmetric
    // border + padding). The label text starts after the icon, the gap and
    // the label wrapper's left padding.
    let content_y = rect.top() + (h - c.m.button.line_height) / 2.0;
    let mut x = rect.left() + (w - content_w) / 2.0;
    if let Some(ic) = icon {
        icon_at(c, ic, pos2(x, content_y), c.m.icon_xs, icon_color);
        x += c.m.icon_xs + c.m.sp4;
    }
    let galley = galley(c.ctx(), c.m.button, label, label_color, f32::INFINITY, false);
    paint_galley(&painter, pos2(x + c.m.sp4, content_y), &galley);

    // Keyboard focus ring (inside the border box, 2px, snapped).
    if ring {
        stroke_focus_ring(&painter, rect, c.m.radius_s, pal.focus_ring, c.ppp());
    }

    // Activate on click / Enter / Space (Space activates on release, like a
    // native button). `clicked_by(Primary)` excludes egui's synthetic
    // keyboard "click", which we handle explicitly below.
    let mut activated = response.clicked_by(egui::PointerButton::Primary);
    if c.focus.is_focused(focus_id) && !disabled {
        let input = c.ctx().input(|i| {
            (
                i.key_pressed(Key::Enter),
                i.key_pressed(Key::Space),
                i.key_released(Key::Space),
            )
        });
        if input.0 {
            activated = true;
        }
        if input.1 {
            c.focus.space_down = Some(focus_id);
        }
        if input.2 && c.focus.space_down == Some(focus_id) {
            activated = true;
            c.focus.space_down = None;
        }
    }

    // Accessibility: expose the button.
    response.widget_info(|| WidgetInfo::labeled(Role::Button, !disabled, label));

    if disabled {
        c.ui.set_cursor_icon(egui::CursorIcon::NotAllowed);
    } else if response.hovered() {
        c.ui.set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    activated
}

/// The width of a button's border box (for row layout before painting).
pub fn button_width(
    ctx: &Context,
    m: &Metrics,
    variant: ButtonVariant,
    label: &str,
    icon: Option<Icon>,
    text_color: Color32,
) -> f32 {
    // The label sits in a `px-4` wrapper (padding-x space-4), so its box is
    // the text plus 2×space-4.
    let label_w = text_width(ctx, m.button, label, text_color) + 2.0 * m.sp4;
    let border_w = if variant == ButtonVariant::Secondary { 1.0 } else { 0.0 };
    let content_w = icon.map(|_| m.icon_xs + m.sp4).unwrap_or(0.0) + label_w;
    content_w + 2.0 * (border_w + m.sp8)
}

/// Linear interpolation between two colors (straight RGBA, like a CSS
/// `transition` on `background-color`).
pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    if t <= 0.0 {
        return a;
    }
    if t >= 1.0 {
        return b;
    }
    let lerp = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(
        lerp(a.r(), b.r()),
        lerp(a.g(), b.g()),
        lerp(a.b(), b.b()),
        lerp(a.a(), b.a()),
    )
}

// ---------------------------------------------------------------------------
// Status dot (StatusIndicator size s)
// ---------------------------------------------------------------------------

/// A StatusIndicator size-s warning dot (space-4 = 3.75px at root 15),
/// centered vertically in `row` at `x`. Returns the dot's width.
pub fn status_dot(c: &mut Ctx, x: f32, row: Rect) -> f32 {
    let d = c.m.sp4;
    let center_y = row.center().y;
    let rect = Rect::from_center_size(pos2(x + d / 2.0, center_y), vec2(d, d));
    c.painter()
        .circle_filled(rect.center(), d / 2.0, c.pal.warning_dot);
    d
}

// ---------------------------------------------------------------------------
// Card / panel frame
// ---------------------------------------------------------------------------

/// Paint a filled, bordered, rounded panel (the request card, the message
/// panel, the conversation panel, the empty state). Returns the content rect
/// (border box inset by the 1px border and `padding`).
pub fn panel_frame(
    c: &mut Ctx,
    rect: Rect,
    radius: f32,
    fill: Color32,
    border: Color32,
    padding: f32,
) -> Rect {
    let painter = c.painter().with_clip_rect(c.ui.clip_rect());
    fill_box(&painter, rect, radius, fill, c.ppp());
    stroke_border(&painter, rect, radius, border, c.ppp());
    rect.shrink(1.0 + padding)
}

// ---------------------------------------------------------------------------
// Row layout (wrap, justify, align-center) for the header / destination /
// buttons rows
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Justify {
    Start,
    End,
    Between,
}

/// Cross-axis alignment of a row's items (CSS `align-items`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowAlign {
    Start,
    Center,
}

/// Wrap a list of item sizes into lines and return each item's rect
/// (top-left based at `origin`, `avail_w` wide, `gap` between items and
/// lines, items aligned per `align` within each line — CSS `align-items`).
/// Each line's height is the tallest item *in that line* (CSS flex
/// `flex-wrap: wrap` line cross sizes). `Justify::Between` is
/// `justify-content: space-between` (a single item stays at the start,
/// like CSS).
pub fn layout_row(
    sizes: &[(f32, f32)],
    origin: Pos2,
    avail_w: f32,
    gap: f32,
    justify: Justify,
    align: RowAlign,
) -> (Vec<Rect>, f32) {
    // Greedy wrap.
    let mut lines: Vec<Vec<usize>> = Vec::new();
    let mut cur: Vec<usize> = Vec::new();
    let mut cur_w = 0.0;
    for (i, (w, _)) in sizes.iter().enumerate() {
        let add = if cur.is_empty() { *w } else { *w + gap };
        if !cur.is_empty() && cur_w + add > avail_w {
            lines.push(std::mem::take(&mut cur));
            cur_w = 0.0;
        }
        cur_w += add;
        cur.push(i);
    }
    if !cur.is_empty() {
        lines.push(cur);
    }

    let mut rects = vec![Rect::ZERO; sizes.len()];
    let mut y = origin.y;
    for line in &lines {
        // CSS flexbox: each wrapped line's cross size is the tallest item
        // *in that line* (not the tallest item overall).
        let line_h = line.iter().map(|&i| sizes[i].1).fold(0.0f32, f32::max);
        let total_w: f32 = line.iter().map(|&i| sizes[i].0).sum::<f32>()
            + gap * (line.len().saturating_sub(1)) as f32;
        let mut x = match justify {
            Justify::Start => origin.x,
            Justify::End => origin.x + (avail_w - total_w).max(0.0),
            Justify::Between => origin.x,
        };
        let extra = if justify == Justify::Between && line.len() > 1 {
            ((avail_w - total_w) / (line.len() - 1) as f32).max(0.0)
        } else {
            0.0
        };
        for &i in line.iter() {
            let (w, h) = sizes[i];
            let iy = match align {
                RowAlign::Start => y,
                RowAlign::Center => y + (line_h - h) / 2.0,
            };
            rects[i] = Rect::from_min_size(pos2(x, iy), vec2(w, h));
            x += w + gap + extra;
        }
        y += line_h + gap;
    }
    let total_h = if lines.is_empty() {
        0.0
    } else {
        y - gap - origin.y
    };
    (rects, total_h)
}

// ---------------------------------------------------------------------------
// Textarea (Once UI Textarea size m, `focusRing`, floating label)
// ---------------------------------------------------------------------------

/// Metrics of the reference's size-m Textarea, resolved per rem root. The
/// root-15 values were captured from the reference's computed styles; they
/// are rem-based in Once UI's field CSS (`--fld-*`), so they scale with the
/// root font size. The 1px border is absolute.
pub mod field {
    use crate::ui::theme::Metrics;

    /// The box's min-height (`--fld-h-base` = space-56 = 52.5).
    pub fn box_min_h(m: &Metrics) -> f32 {
        m.sp56
    }
    /// The inner well's padding (space-4 = 3.75).
    pub fn well(m: &Metrics) -> f32 {
        m.sp4
    }
    /// The textarea element's padding (label floating: it always is, because
    /// the field always has a placeholder).
    pub fn text_pad_top(m: &Metrics) -> f32 {
        16.2087 * m.s
    }
    pub fn text_pad_bottom(m: &Metrics) -> f32 {
        4.29125 * m.s
    }
    pub fn text_pad_x(m: &Metrics) -> f32 {
        m.sp12
    }
    /// The floating label: `label-default-m` scaled 0.75, positioned against
    /// the well (top = --fld-label-top = 9.13062, left = --fld-inset = 15).
    pub const LABEL_SCALE: f32 = 0.75;
    pub fn label_top(m: &Metrics) -> f32 {
        9.13062 * m.s
    }
    pub fn label_left(m: &Metrics) -> f32 {
        m.sp16
    }
    /// The text line height (`--fld-lh` = line-height body-m = 22.5).
    pub fn line_h(m: &Metrics) -> f32 {
        m.field.line_height
    }

    /// The box height for `lines` lines of text (auto-grow).
    pub fn box_height(m: &Metrics, lines: usize) -> f32 {
        box_min_h(m) + (lines.saturating_sub(1)) as f32 * line_h(m)
    }
}

/// The textarea's outcome.
pub struct TextareaOutput {
    /// True when the text changed this frame.
    pub changed: bool,
    /// True when the field has (egui) focus.
    pub focused: bool,
    /// The text-edit's widget id (for requesting focus).
    pub id: Id,
    /// The box's border-box rect.
    pub rect: Rect,
}

/// A Once UI Textarea (size m, `focusRing`, floating label, placeholder,
/// auto-grow). `text` is owned by the caller; the field is keyed by `id`
/// (pass a per-request id so the edit state resets with the request).
#[allow(clippy::too_many_arguments)]
pub fn textarea(
    c: &mut Ctx,
    id: Id,
    pos: Pos2,
    width: f32,
    text: &mut String,
    label: &str,
    placeholder: &str,
    disabled: bool,
) -> TextareaOutput {
    let pal = c.pal;
    let m = c.m;
    // The textarea is focusable: include it in the DOM (tab) order. A disabled
    // textarea is skipped, like the browser skips disabled controls.
    if !disabled {
        c.focus.register(FocusId::Textarea);
    }
    // Measure the current text to size the box (auto-grow).
    let text_w = width - 2.0 * (1.0 + field::well(m) + field::text_pad_x(m));
    let measure = galley(c.ctx(), m.field, text, pal.text_strong, text_w, false);
    let lines = measure.rows.len().max(1);
    let box_h = field::box_height(m, lines);
    let rect = Rect::from_min_size(pos, vec2(width, box_h));

    let painter = c.painter().with_clip_rect(c.ui.clip_rect());

    // The box: fill, border, focus ring.
    fill_box(&painter, rect, m.radius_l, pal.field_bg, c.ppp());
    let focused = c.ctx().memory(|mem| mem.has_focus(id));
    let border_color = if focused {
        pal.field_border_focused
    } else {
        pal.field_border
    };
    stroke_border(&painter, rect, m.radius_l, border_color, c.ppp());
    if focused {
        stroke_field_ring(&painter, rect, m.radius_l, pal.brand_solid_strong, c.ppp());
    }

    // The well (inner column): the border box inset by the 1px border.
    let well = Rect::from_min_size(
        pos2(rect.left() + 1.0, rect.top() + 1.0),
        vec2(width - 2.0, box_h - 2.0),
    );

    // The floating label (always floating: the field has a placeholder).
    let label_spec = TextSpec {
        size: m.label_m.size * field::LABEL_SCALE,
        weight: m.label_m.weight,
        line_height: m.label_m.line_height * field::LABEL_SCALE,
        letter_spacing: m.label_m.letter_spacing,
        mono: false,
    };
    let label_galley = galley(
        c.ctx(),
        label_spec,
        label,
        pal.text_medium,
        f32::INFINITY,
        false,
    );
    paint_galley(
        &painter,
        pos2(
            well.left() + field::label_left(m),
            well.top() + field::label_top(m),
        ),
        &label_galley,
    );

    // The text area (the textarea element: well inset by its padding, with
    // the floating label's space on top). The TextEdit paints its galley at
    // the widget rect's top-left, so the layouter below hands it a galley
    // whose rows are snapped to the reference's rendered baselines for this
    // exact line top; the placeholder is painted through `paint_galley`
    // (which applies the same correction itself).
    let text_pos = pos2(
        well.left() + field::well(m) + field::text_pad_x(m),
        well.top() + field::well(m) + field::text_pad_top(m),
    );
    let text_rect = Rect::from_min_size(
        text_pos,
        vec2(
            width - 2.0 * (1.0 + field::well(m) + field::text_pad_x(m)),
            lines as f32 * field::line_h(m) + field::text_pad_bottom(m),
        ),
    );

    // Clicking anywhere in the well focuses the field.
    let click_layer = c.ui.interact(well, id.with("click"), Sense::click());
    if click_layer.clicked_by(egui::PointerButton::Primary) && !disabled {
        c.ctx().memory_mut(|m| m.request_focus(id));
        c.focus.focus(FocusId::Textarea, false);
    }
    if click_layer.is_pointer_button_down_on() {
        c.press_consumed = true;
    }

    // The TextEdit itself, placed at the text area.
    let text_color = if disabled {
        pal.text_weak
    } else {
        pal.text_strong
    };
    let mut child = c
        .ui
        .new_child(egui::UiBuilder::new().max_rect(text_rect));
    let out = child.add(
        egui::TextEdit::multiline(text)
            .id(id)
            .frame(Frame::NONE)
            .desired_width(text_rect.width())
            .min_size(text_rect.size())
            // egui's multiline default is 4 rows, which would make the
            // widget's hit rect extend well past the field; size it to the
            // text like the reference's auto-growing textarea.
            .desired_rows(lines)
            .layouter(&mut |ui, buf, wrap_width| {
                let job = m.field.job(buf.as_str(), text_color, wrap_width, false);
                let galley = ui.fonts_mut(|f| f.layout_job(job));
                crate::ui::text::snapped_galley(&galley, text_pos.y)
            })
            .text_color(text_color)
            .interactive(!disabled)
            .cursor_at_end(false)
            .hint_text(""),
    );
    // A press that starts inside the field is consumed by the field (the
    // browser focuses the textarea and keeps the focus). The TextEdit is
    // created after the click layer, so the press lands on the TextEdit
    // (the topmost widget), not the layer — and egui clears
    // `is_pointer_button_down_on` when the press and release land in the
    // same frame. Test the press position against the well instead.
    let pressed_in_field = c.ctx().input(|i| {
        i.pointer.primary_pressed()
            && i.pointer
                .latest_pos()
                .map(|p| well.contains(p))
                .unwrap_or(false)
    });
    if pressed_in_field {
        c.press_consumed = true;
    }
    if out.has_focus() {
        c.focus.focus(FocusId::Textarea, c.focus.keyboard);
    }

    // Placeholder (painted by us, over the empty field).
    if text.is_empty() && !disabled {
        let ph = galley(
            c.ctx(),
            m.field,
            placeholder,
            pal.placeholder,
            text_rect.width(),
            false,
        );
        paint_galley(&painter, text_pos, &ph);
    }

    let changed = out.changed();
    TextareaOutput {
        changed,
        focused,
        id,
        rect,
    }
}

// ---------------------------------------------------------------------------
// Text-effect title (per-word reveal; the blurred phase is painted by the
// wgpu callback in shaders.rs — see app.rs)
// ---------------------------------------------------------------------------

/// One laid-out title segment (word or whitespace run).
pub struct TitleSegment {
    pub text: String,
    /// The segment's galley (`font-display font-strong font-m`).
    pub galley: Arc<Galley>,
    /// The segment's width including the trailing letter-spacing (CSS adds
    /// letter-spacing after every character, including the last).
    pub advance: f32,
}

/// Lay out the title's segments (split like the reference's `per="word"`)
/// in the resolved `font-display font-strong font-m` spec.
pub fn title_segments(ctx: &Context, m: &Metrics, text: &str, color: Color32) -> Vec<TitleSegment> {
    let effect = crate::ui::anim::TextEffect::new(text);
    effect
        .segments()
        .iter()
        .map(|seg| {
            let galley = galley(ctx, m.title, seg, color, f32::INFINITY, false);
            // The segment's advance as Chrome renders it: the galley width
            // plus the trailing unit (CSS letter-spacing and CoreText's
            // `trak` tracking both apply after the last glyph too).
            let advance =
                galley.rect.width() + m.title.letter_spacing + m.title.trak_tracking();
            TitleSegment {
                text: seg.clone(),
                galley,
                advance,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Small helpers used by app.rs
// ---------------------------------------------------------------------------

/// Section label (`label-default-xs`, text_medium).
pub fn section_label(c: &mut Ctx, pos: Pos2, text: &str) -> Rect {
    text_at(
        c,
        pos,
        c.m.label_xs,
        text,
        c.pal.text_medium,
        f32::INFINITY,
        false,
    )
}

/// Section hint (`body-default-xs`, text_medium).
pub fn hint_text(c: &mut Ctx, pos: Pos2, text: &str) -> Rect {
    text_at(
        c,
        pos,
        c.m.body_xs,
        text,
        c.pal.text_medium,
        f32::INFINITY,
        false,
    )
}

/// `body-default-s` selectable text.
pub fn body_text(
    c: &mut Ctx,
    id: Id,
    pos: Pos2,
    text: &str,
    color: Color32,
    wrap_width: f32,
) -> Rect {
    selectable_text(c, id, pos, c.m.body_s, text, color, wrap_width, false)
}

/// `code-default-s` selectable monospace text (the proposed message).
pub fn code_text(
    c: &mut Ctx,
    id: Id,
    pos: Pos2,
    text: &str,
    color: Color32,
    wrap_width: f32,
) -> Rect {
    selectable_text(c, id, pos, c.m.code_s, text, color, wrap_width, true)
}

/// `body-strong-s` plain text (the conversation title).
pub fn body_strong_text(c: &mut Ctx, pos: Pos2, text: &str, color: Color32, wrap_width: f32) -> Rect {
    text_at(c, pos, c.m.body_strong_s, text, color, wrap_width, false)
}

/// `code-default-xs` selectable monospace text (the conversation link).
pub fn code_xs_text(
    c: &mut Ctx,
    id: Id,
    pos: Pos2,
    text: &str,
    color: Color32,
    wrap_width: f32,
) -> Rect {
    selectable_text(c, id, pos, c.m.code_xs, text, color, wrap_width, true)
}

/// `body-strong-m` plain text (the empty-state title).
pub fn body_strong_m_text(c: &mut Ctx, pos: Pos2, text: &str, color: Color32) -> Rect {
    text_at(c, pos, c.m.body_strong_m, text, color, f32::INFINITY, false)
}

/// `display-strong-s` plain text (the card heading).
pub fn heading_text(c: &mut Ctx, pos: Pos2, text: &str) -> Rect {
    text_at(
        c,
        pos,
        c.m.heading,
        text,
        c.pal.text_strong,
        f32::INFINITY,
        false,
    )
}
