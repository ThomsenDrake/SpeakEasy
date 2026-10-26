//! Typography: the reference's text variants and the layout helpers used to
//! measure and paint them.
//!
//! Sizes/weights/line-heights/letter-spacings are the computed styles of
//! the reference app (Once UI `@once-ui-system/core` 2.5.0 typography
//! classes: `display-strong-s`, `body-default-s`, `label-default-xs`,
//! `font-code`, `font-label`, …). All "default" weights render at wght 300
//! (Once UI's `.font-default` falls back to the display-default weight),
//! "strong" at 600.

use epaint::text::{
    ByteIndex, Galley, LayoutJob, LayoutSection, PlacedRow, TextFormat, TextWrapping, VariationCoords,
};
use egui::{pos2, Align, Color32, Context, FontFamily, FontId, Pos2, Rect, Stroke, Vec2};
use std::sync::Arc;

/// One text variant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextSpec {
    /// Font size in px (the reference root is 15px; these are absolute).
    pub size: f32,
    /// `wght` variation coordinate (300 = "default", 600 = "strong").
    pub weight: f32,
    /// CSS line-height in px.
    pub line_height: f32,
    /// CSS letter-spacing in px.
    pub letter_spacing: f32,
    /// Monospace family (Menlo) instead of proportional (SF Pro).
    pub mono: bool,
}

impl TextSpec {
    pub const fn new(size: f32, weight: f32, line_height: f32, letter_spacing: f32) -> Self {
        Self {
            size,
            weight,
            line_height,
            letter_spacing,
            mono: false,
        }
    }

    pub const fn mono(size: f32, weight: f32, line_height: f32) -> Self {
        Self {
            size,
            weight,
            line_height,
            letter_spacing: 0.0,
            mono: true,
        }
    }

    pub fn font_id(&self) -> FontId {
        FontId {
            size: self.size,
            family: if self.mono {
                FontFamily::Monospace
            } else {
                FontFamily::Proportional
            },
        }
    }

    /// The per-glyph horizontal tracking CoreText adds to SF Pro text at this
    /// spec's size, in px. Returns 0 for the monospace family (Menlo has no
    /// `trak` table).
    ///
    /// SF Pro (`SFNS.ttf`) carries an Apple `trak` table that CoreText applies
    /// automatically: the default track (track value 0.0) adds a size-dependent
    /// tracking value to every glyph advance. The reference app is rendered by
    /// Chrome, which shapes through CoreText, so its glyph advances include
    /// this tracking; egui's shaper (harfrust) does not implement `trak`, and
    /// without a correction every glyph position drifts by up to ±0.4px
    /// (measured: Chrome − egui per-glyph advance is exactly the `trak`
    /// value at every measured size and weight, e.g. +9 units at 11.25px,
    /// −40 at 16px, +27 at 30px).
    ///
    /// The tracking values below are the raw F2Dot14 integers of the default
    /// track, which CoreText applies as font units (SF Pro's upem is 2048).
    /// They are linearly interpolated between the table's size breakpoints.
    pub fn trak_tracking(&self) -> f32 {
        if self.mono {
            return 0.0;
        }
        const BREAKPOINTS: &[(f32, f32)] = &[
            (6.0, 82.0),
            (9.0, 38.0),
            (10.0, 24.0),
            (11.0, 12.0),
            (12.0, 0.0),
            (13.0, -12.0),
            (14.0, -22.0),
            (15.0, -32.0),
            (16.0, -40.0),
            (17.0, -52.0),
            (20.0, -46.0),
            (22.0, -24.0),
            (24.0, 6.0),
            (28.0, 28.0),
            (32.0, 26.0),
            (36.0, 21.0),
            (50.0, 14.0),
            (64.0, 7.0),
            (80.0, 0.0),
            (100.0, 0.0),
            (138.0, 0.0),
        ];
        let size = self.size;
        let units = match BREAKPOINTS.iter().position(|&(bp, _)| size < bp) {
            // Below the first breakpoint: the first value.
            None => BREAKPOINTS[BREAKPOINTS.len() - 1].1,
            Some(0) => BREAKPOINTS[0].1,
            Some(i) => {
                let (s0, v0) = BREAKPOINTS[i - 1];
                let (s1, v1) = BREAKPOINTS[i];
                v0 + (v1 - v0) * (size - s0) / (s1 - s0)
            }
        };
        units * size / 2048.0
    }

    /// The same variant at a different rem root: Once UI's body/label/code
    /// sizes and line-heights are rem-based, so they scale linearly with the
    /// root font size (15px at viewport widths ≤768px, 16px above).
    pub fn scaled(&self, s: f32) -> Self {
        Self {
            size: self.size * s,
            weight: self.weight,
            line_height: self.line_height * s,
            letter_spacing: self.letter_spacing * s,
            mono: self.mono,
        }
    }

    pub fn format(&self, color: Color32) -> TextFormat {
        // Variation coordinates matching what Chrome renders for this style:
        // the CSS `font-weight` maps to `wght`, and `font-optical-sizing:
        // auto` (the CSS default) maps the used size to SF Pro's `opsz` axis
        // (clamped to the axis range 17–96). Menlo has no `opsz` axis, so
        // only `wght` applies there.
        let coords = if self.mono {
            VariationCoords::new([(b"wght", self.weight)])
        } else {
            VariationCoords::new([
                (b"wght", self.weight),
                (b"opsz", self.size.clamp(17.0, 96.0)),
            ])
        };
        TextFormat {
            font_id: self.font_id(),
            // CoreText (and therefore Chrome) adds SF Pro's `trak` tracking
            // to every glyph's advance; egui's shaper (harfrust) does not
            // implement `trak`. Adding it as letter-spacing reproduces the
            // reference's glyph positions exactly (see `coretext_trak_tracking`).
            extra_letter_spacing: self.letter_spacing + self.trak_tracking(),
            line_height: Some(self.line_height),
            color,
            background: Color32::TRANSPARENT,
            expand_bg: 1.0,
            coords,
            italics: false,
            underline: Stroke::NONE,
            strikethrough: Stroke::NONE,
            valign: Align::BOTTOM,
        }
    }

    /// A layout job for `text` in this style, wrapping at `wrap_width`
    /// (`f32::INFINITY` for no wrap); `\n` always breaks. `break_anywhere` is
    /// egui's native flag (it breaks an over-long word at the last character
    /// that fits the *current row's remaining width* — not CSS
    /// `overflow-wrap: anywhere` semantics; use [`galley_anywhere`] for
    /// those).
    pub fn job(&self, text: &str, color: Color32, wrap_width: f32, break_anywhere: bool) -> LayoutJob {
        self.job_align(text, color, wrap_width, break_anywhere, Align::LEFT)
    }

    /// Like [`Self::job`], with an explicit horizontal alignment (egui
    /// centers each row on the galley's origin, so a centered galley is
    /// painted at its *center* x).
    pub fn job_align(
        &self,
        text: &str,
        color: Color32,
        wrap_width: f32,
        break_anywhere: bool,
        halign: Align,
    ) -> LayoutJob {
        let format = self.format(color);
        LayoutJob {
            text: text.to_owned(),
            sections: vec![LayoutSection {
                leading_space: 0.0,
                byte_range: ByteIndex(0)..ByteIndex(text.len()),
                format,
            }],
            wrap: TextWrapping {
                max_width: wrap_width,
                max_rows: usize::MAX,
                break_anywhere,
                overflow_character: Some('…'),
            },
            first_row_min_height: 0.0,
            break_on_newline: true,
            halign,
            justify: false,
            // Chrome subpixel-positions glyphs; don't round the output.
            round_output_to_gui: false,
            keep_trailing_whitespace: false,
        }
    }
}

/// `display-strong-s` — the request-card heading.
pub const DISPLAY_STRONG_S: TextSpec = TextSpec::new(30.0, 600.0, 37.5, -0.6);
/// `font-display font-strong font-m` — the header title.
pub const TITLE_M: TextSpec = TextSpec::new(33.75, 600.0, 37.5, -1.0125);
/// `body-default-s`.
pub const BODY_DEFAULT_S: TextSpec = TextSpec::new(13.125, 300.0, 16.875, 0.0);
/// `body-strong-s`.
pub const BODY_STRONG_S: TextSpec = TextSpec::new(13.125, 600.0, 16.875, 0.0);
/// `body-default-xs`.
pub const BODY_DEFAULT_XS: TextSpec = TextSpec::new(11.25, 300.0, 15.0, 0.0);
/// `body-strong-m`.
pub const BODY_STRONG_M: TextSpec = TextSpec::new(15.0, 600.0, 22.5, 0.0);
/// `label-default-xs` — section labels.
pub const LABEL_DEFAULT_XS: TextSpec = TextSpec::new(11.25, 300.0, 13.125, 0.0);
/// `label-default-s` — tag and badge labels.
pub const LABEL_DEFAULT_S: TextSpec = TextSpec::new(12.375, 300.0, 15.0, 0.0);
/// `label-default-m` — the textarea label.
pub const LABEL_DEFAULT_M: TextSpec = TextSpec::new(13.875, 300.0, 18.75, 0.0);
/// `font-label font-s font-strong` — button labels.
pub const BUTTON_LABEL: TextSpec = TextSpec::new(12.375, 600.0, 15.0, 0.0);
/// `code-default-s` — the proposed-message preview.
pub const CODE_DEFAULT_S: TextSpec = TextSpec::mono(12.375, 300.0, 15.0);
/// `code-default-xs` — the conversation link.
pub const CODE_DEFAULT_XS: TextSpec = TextSpec::mono(11.25, 300.0, 13.125);
/// The textarea value text (`body-default`, root size).
pub const FIELD_TEXT: TextSpec = TextSpec::new(15.0, 300.0, 22.5, 0.0);

/// Lay out `text` and return the galley. The galley is in *points*; multiply
/// positions by `pixels_per_point` for physical pixels.
pub fn layout(ctx: &Context, job: LayoutJob) -> Arc<Galley> {
    ctx.fonts_mut(|fonts| fonts.layout_job(job))
}

/// The size of a laid-out galley.
pub fn galley_size(galley: &Galley) -> Vec2 {
    galley.rect.size()
}

// ---------------------------------------------------------------------------
// CSS `overflow-wrap: anywhere` line breaking
//
// The reference wraps two texts with `overflow-wrap: anywhere` (the proposed
// message, `white-space: pre-wrap`, and the conversation link). Chrome's
// line breaking for that is: fill the line greedily and break at the last
// break opportunity (a space, hyphen, solidus, …) that fits; a word that
// does not fit the *remaining* width moves to the next line whole; only a
// word wider than a full line is broken, at the last character that fits
// (the `anywhere` opportunities apply only when the line has no other
// acceptable break point). egui's native `TextWrapping::break_anywhere`
// instead fills the remaining width of a row with the start of an over-long
// word, which the reference never does — so the lines are computed here and
// laid out separately, then assembled into one galley (egui tessellates
// each row independently, so the per-line rows can be stacked as they are).
// ---------------------------------------------------------------------------

/// One visual line of an anywhere-wrapped text: a char range of the
/// original text, and whether the line ends at a hard `\n` (the `\n` itself
/// is not part of any row's glyphs).
struct CssLine {
    start: usize,
    end: usize,
    hard_break: bool,
}

/// Whether a line may break *after* this char (the in-word break
/// opportunities Chrome uses): whitespace, the hyphen-minus, the solidus
/// and the em dash. The period is *not* one — Chrome does not break
/// "J9.eyJ" after the `.`.
fn is_break_opportunity(ch: char) -> bool {
    ch.is_whitespace() || ch == '-' || ch == '/' || ch == '\u{2014}'
}

/// Chrome's line breaks for `overflow-wrap: anywhere` (see the module docs
/// above). `chars`/`adv` are the text's chars and their shaped advances
/// (one entry per char; `\n` entries are ignored — they are hard breaks).
/// `extra` is the letter-spacing egui adds before every glyph but a line's
/// first one.
fn css_anywhere_lines(chars: &[char], adv: &[f32], extra: f32, max_width: f32) -> Vec<CssLine> {
    let n = chars.len();
    let mut lines = Vec::new();
    let mut line_start = 0;
    let mut x = 0.0; // the width of the current line, as egui will paint it
    let mut last_opp: Option<usize> = None; // char index just after a break opportunity
    let mut k = 0;
    while k < n {
        if chars[k] == '\n' {
            lines.push(CssLine { start: line_start, end: k, hard_break: true });
            k += 1;
            line_start = k;
            x = 0.0;
            last_opp = None;
            continue;
        }
        let step = adv[k] + if k > line_start { extra } else { 0.0 };
        if x + step > max_width {
            // The line overflows at char k: break at the last break
            // opportunity in the line or — with no acceptable break point
            // in the line — at the last character that fits.
            let break_at = match last_opp {
                Some(o) if o > line_start => o,
                _ if k > line_start => k,
                _ => k + 1, // one char wider than the line: overrun it
            };
            lines.push(CssLine { start: line_start, end: break_at, hard_break: false });
            let count = k - break_at;
            x = adv[break_at..k].iter().sum::<f32>() + extra * count.saturating_sub(1) as f32;
            line_start = break_at;
            last_opp = None;
            continue; // re-place char k on the new line
        }
        x += step;
        if is_break_opportunity(chars[k]) {
            last_opp = Some(k + 1);
        }
        k += 1;
    }
    lines.push(CssLine { start: line_start, end: n, hard_break: false });
    lines
}

/// Lay out `text` at `wrap_width` with CSS `overflow-wrap: anywhere`
/// semantics (Chrome's line breaking; see the module docs above). The
/// returned galley keeps the full text as its job — so text selection,
/// cursor mapping and copy behave exactly like the reference — and its rows
/// are the per-line rows stacked at the style's (pixel-rounded) line height.
pub fn galley_anywhere(
    ctx: &Context,
    spec: TextSpec,
    text: &str,
    color: Color32,
    wrap_width: f32,
) -> Arc<Galley> {
    let native = |break_anywhere: bool| layout(ctx, spec.job(text, color, wrap_width, break_anywhere));
    // Shape the whole text once, without wrapping, to measure the per-char
    // advances exactly as they will be painted.
    let full = layout(ctx, spec.job(text, color, f32::INFINITY, false));
    let extra = spec.letter_spacing + spec.trak_tracking();
    let mut chars: Vec<char> = Vec::with_capacity(text.chars().count());
    let mut adv: Vec<f32> = Vec::with_capacity(chars.capacity());
    let mut glyphs = full.rows.iter().flat_map(|row| row.glyphs.iter());
    for ch in text.chars() {
        if ch == '\n' {
            chars.push(ch);
            adv.push(0.0);
            continue;
        }
        match glyphs.next() {
            Some(g) if g.chr == ch => {
                chars.push(ch);
                adv.push(g.advance_width);
            }
            // The shaper did not produce exactly one glyph per char
            // (ligatures, zero-width chars): keep egui's native wrapping.
            _ => return native(true),
        }
    }
    if glyphs.next().is_some() {
        return native(true);
    }
    let lines = css_anywhere_lines(&chars, &adv, extra, wrap_width);
    // The byte offset of each char, for slicing the lines.
    let mut byte_of: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
    byte_of.push(text.len());
    // Lay out every visual line on its own (no wrap, no `\n` inside) and
    // stack the rows.
    let mut rows: Vec<PlacedRow> = Vec::with_capacity(lines.len());
    let mut row_h = 0.0;
    let mut template: Option<Galley> = None;
    for (k, line) in lines.iter().enumerate() {
        let slice = &text[byte_of[line.start]..byte_of[line.end]];
        let line_galley = layout(ctx, spec.job(slice, color, f32::INFINITY, false));
        let Some(pr) = line_galley.rows.first() else {
            return native(true);
        };
        let h = pr.row.size.y;
        if k == 0 {
            row_h = h;
        } else if (h - row_h).abs() > 1e-4 {
            return native(true);
        }
        rows.push(PlacedRow {
            pos: pos2(0.0, k as f32 * row_h),
            row: pr.row.clone(),
            ends_with_newline: line.hard_break,
        });
        if template.is_none() {
            template = Some((*line_galley).clone());
        }
    }
    let Some(mut assembled) = template else {
        return native(false);
    };
    // The galley describes the full text (selection, copy, cursor mapping).
    // `intrinsic_size` stays from the template: it is only read when a text
    // shape is scaled, which this app never does.
    assembled.job = Arc::new(spec.job(text, color, wrap_width, true));
    assembled.rows = rows;
    assembled.elided = false;
    let mut rect = Rect::ZERO;
    let mut mesh_bounds = Rect::NOTHING;
    let mut num_vertices = 0;
    let mut num_indices = 0;
    for pr in &assembled.rows {
        rect |= pr.rect();
        mesh_bounds |= pr.row.visuals.mesh_bounds.translate(pr.pos.to_vec2());
        num_vertices += pr.row.visuals.mesh.vertices.len();
        num_indices += pr.row.visuals.mesh.indices.len();
    }
    assembled.rect = rect;
    assembled.mesh_bounds = mesh_bounds;
    assembled.num_vertices = num_vertices;
    assembled.num_indices = num_indices;
    Arc::new(assembled)
}

// ---------------------------------------------------------------------------
// Baseline placement
//
// The reference is rendered by Chrome, whose line boxes place the baseline
// from the font's CoreText metrics *rounded to whole pixels*: with
// `a = round(ascent)` and `d = round(descent)`, the first-line baseline sits
// `a + floor((line_height - (a + d)) / 2)` below the line-box top. The offset
// is therefore always a whole number of px (at device scale 1 and 2 alike).
// egui instead derives the baseline from the font's raw metrics (variation
// deltas not applied) and snaps it to whole *device* pixels, which lands up
// to ~1px away from the reference. `baseline_correction` measures the
// difference per galley and the paint helpers apply it, reproducing the
// reference's baselines exactly.
// ---------------------------------------------------------------------------

/// CoreText's ascent/descent (px) for the app's fonts at the sizes the app
/// uses, measured with `CTFontGetAscent`/`CTFontGetDescent` (SF Pro's metrics
/// are MVAR-varied at `opsz = clamp(size, 17, 96)` — CoreText's optical
/// sizing — and identical for wght 300 and 600; Menlo is static). Sizes
/// between entries interpolate linearly (the metrics are linear in size
/// within each optical-sizing segment).
fn coretext_metrics(size: f32, mono: bool) -> (f32, f32) {
    const SF: &[(f32, f32, f32)] = &[
        // (size, ascent, descent)
        (10.40625, 10.060730, 2.195068),
        (11.1, 10.731445, 2.341406),
        (11.25, 10.876465, 2.373047),
        (12.0, 11.601562, 2.531250),
        (12.375, 11.964111, 2.610352),
        (13.125, 12.689209, 2.768555),
        (13.2, 12.761719, 2.784375),
        (13.875, 13.414307, 2.926758),
        (14.0, 13.535156, 2.953125),
        (14.8, 14.308594, 3.121875),
        (15.0, 14.501953, 3.164062),
        (16.0, 15.468750, 3.375000),
        (30.0, 29.003906, 6.328125),
        (32.0, 30.937500, 6.750000),
        (33.75, 32.629395, 7.119141),
        (36.0, 34.804688, 7.593750),
        // The display sizes Once UI steps to at the ≥769px and ≥1025px
        // breakpoints (title 2.75rem/3rem, heading 2.25rem/2.5rem — the
        // metrics are linear in size, so the entries sit exactly on the
        // 36→48 line).
        (40.0, 38.671875, 8.437500),
        (44.0, 42.539062, 9.281250),
        (48.0, 46.406250, 10.125000),
    ];
    const MENLO: &[(f32, f32, f32)] = &[
        (11.25, 10.442505, 2.653198),
        (12.0, 11.138672, 2.830078),
        (12.375, 11.486755, 2.918518),
        (13.2, 12.252539, 3.113086),
    ];
    let table: &[(f32, f32, f32)] = if mono { MENLO } else { SF };
    if let Some(&(_, a, d)) = table.iter().find(|&&(s, _, _)| s == size) {
        return (a, d);
    }
    // Interpolate between the neighbouring sizes.
    let i = table
        .iter()
        .position(|&(s, _, _)| s > size)
        .unwrap_or(table.len());
    match i {
        0 => (table[0].1, table[0].2),
        n if n >= table.len() => {
            let &(_, a, d) = table.last().unwrap();
            (a, d)
        }
        n => {
            let &(s0, a0, d0) = &table[n - 1];
            let &(s1, a1, d1) = &table[n];
            let t = (size - s0) / (s1 - s0);
            (a0 + (a1 - a0) * t, d0 + (d1 - d0) * t)
        }
    }
}

/// The reference's first-line baseline offset from the line-box top, in px:
/// `round(ascent) + floor((line_height - (round(ascent) + round(descent))) / 2)`.
pub fn chrome_baseline_offset(size: f32, mono: bool, line_height: f32) -> f32 {
    let (a, d) = coretext_metrics(size, mono);
    let a = a.round();
    let d = d.round();
    // The reference's line-heights are multiples of 1/16 px; snap away the
    // float wobble the rem scaling (×16/15 in f32) introduces so the floor
    // below lands on the same side as the reference's exact arithmetic.
    let line_height = (line_height * 16.0).round() / 16.0;
    a + ((line_height - (a + d)) / 2.0).floor()
}

impl TextSpec {
    /// The reference's first-line baseline offset from the line-box top.
    pub fn chrome_baseline_offset(&self) -> f32 {
        chrome_baseline_offset(self.size, self.mono, self.line_height)
    }
}

/// The first row's baseline offset from the galley's top, exactly where egui
/// will paint the glyphs (before any correction): `row.pos.y + glyph.pos.y`.
pub fn galley_baseline(galley: &Galley) -> f32 {
    match galley.rows.first() {
        Some(row) => match row.glyphs.first() {
            Some(glyph) => row.pos.y + glyph.pos.y,
            None => row.pos.y,
        },
        None => 0.0,
    }
}

/// The `(size, monospace, line_height)` a galley was laid out with (recovered
/// from its layout job; the baseline offset is weight-independent).
fn galley_style(galley: &Galley) -> Option<(f32, bool, f32)> {
    let format = &galley.job.sections.first()?.format;
    let mono = format.font_id.family == FontFamily::Monospace;
    Some((format.font_id.size, mono, format.line_height?))
}

/// The vertical correction that places a galley's first-line baseline where
/// the reference (Chrome) renders it. Chrome places the line box baseline at
/// [`chrome_baseline_offset`] below the line-box top and then snaps the
/// *rendered* baseline to whole CSS pixels (`round` — verified at device
/// scales 1 and 2, for both fonts, across fractional line tops; halves round
/// up). egui instead lays the baseline out from the font's raw metrics
/// (variation deltas not applied) and snaps it to whole *device* pixels.
/// Paint the galley at `pos + vec2(0.0, baseline_correction(galley, pos.y))`.
///
/// This only fixes the first line. For later lines use
/// [`row_baseline_shifts`]: egui also snaps the row *pitch* to whole device
/// pixels (16.875 → 17.0 at 2×), while Chrome stacks line boxes at the exact
/// fractional line-height and snaps each rendered baseline individually.
pub fn baseline_correction(galley: &Galley, line_top: f32) -> f32 {
    match galley_style(galley) {
        Some((size, mono, line_height)) => {
            let target = chrome_baseline_offset(size, mono, line_height);
            (line_top + target).round() - (line_top + galley_baseline(galley))
        }
        None => 0.0,
    }
}

/// The per-row vertical shifts (CSS px) that place every row's rendered
/// baseline where the reference renders it: Chrome stacks line boxes at the
/// exact fractional line-height and snaps each *rendered* baseline to whole
/// CSS px — `round(line_top + k*line_height + baseline_offset)`. egui snaps
/// both the row pitch and the baselines to whole device pixels, so from the
/// second line on a row can sit up to a whole pixel off. Returns `None` when
/// painting at `line_top + baseline_correction(galley, line_top)` already
/// lands every row correctly (single-row galleys, and multi-row galleys whose
/// snapped pitch is exact, e.g. an integral line-height).
pub fn row_baseline_shifts(galley: &Galley, line_top: f32) -> Option<Vec<f32>> {
    let (size, mono, line_height) = galley_style(galley)?;
    // Snap away the float wobble the rem scaling introduces (the reference's
    // line-heights are multiples of 1/16 px), as in `chrome_baseline_offset`.
    let line_height = (line_height * 16.0).round() / 16.0;
    let target = chrome_baseline_offset(size, mono, line_height);
    let dy = baseline_correction(galley, line_top);
    let mut shifts = Vec::with_capacity(galley.rows.len());
    for (k, row) in galley.rows.iter().enumerate() {
        let shift = match row.glyphs.first() {
            Some(glyph) => {
                let want = (line_top + k as f32 * line_height + target).round();
                want - (line_top + row.pos.y + glyph.pos.y)
            }
            // Empty row (e.g. trailing newline): nothing is painted; shift it
            // like the first row so selection rects stay consistent.
            None => dy,
        };
        shifts.push(shift);
    }
    if shifts.iter().all(|&s| (s - dy).abs() < 1e-3) {
        None
    } else {
        Some(shifts)
    }
}

/// A galley whose rows are shifted so that, painted at `line_top` (the
/// line-box top, unshifted), every row's rendered baseline lands where the
/// reference renders it (see [`row_baseline_shifts`]). In the common case
/// (no per-row adjustment needed) this is just an `Arc` clone of `galley`.
pub fn snapped_galley(galley: &Arc<Galley>, line_top: f32) -> Arc<Galley> {
    match row_baseline_shifts(galley, line_top) {
        None => galley.clone(),
        Some(shifts) => {
            let mut snapped = (**galley).clone();
            for (row, &shift) in snapped.rows.iter_mut().zip(&shifts) {
                row.pos.y += shift;
            }
            Arc::new(snapped)
        }
    }
}

/// Paint a galley at `pos` (its top-left, the line-box top), with every
/// line's baseline placed where the reference (Chrome) renders it. Clipping
/// comes from the painter's clip rect (`Painter::with_clip_rect`).
pub fn paint_galley(painter: &egui::Painter, pos: Pos2, galley: &Arc<Galley>) {
    if row_baseline_shifts(galley, pos.y).is_some() {
        let snapped = snapped_galley(galley, pos.y);
        painter.add(egui::Shape::galley(pos, snapped, Color32::TRANSPARENT));
    } else {
        let dy = baseline_correction(galley, pos.y);
        painter.add(egui::Shape::galley(
            egui::pos2(pos.x, pos.y + dy),
            galley.clone(),
            Color32::TRANSPARENT,
        ));
    }
}

/// Install the fonts the reference uses: SF Pro (proportional), Menlo
/// (monospace — what the reference's mono stack resolves to in Chrome on
/// macOS) and Apple Color Emoji (color emoji fallback). On non-macOS
/// systems the files are absent and the eframe `system_fonts` provider
/// (fontique/CoreText) resolves system fonts instead.
///
/// Must be called before the first frame (from the eframe `AppCreator`).
pub fn install_fonts(ctx: &Context) {
    let mut defs = egui::FontDefinitions::default();
    let mut proportional: Vec<String> = Vec::new();
    let mut monospace: Vec<String> = Vec::new();

    let sys = |path: &str| -> Option<Vec<u8>> {
        match std::fs::read(path) {
            Ok(bytes) => Some(bytes),
            Err(e) => {
                eprintln!("slack-approval-gateway: cannot read system font {path}: {e}");
                None
            }
        }
    };

    if let Some(bytes) = sys("/System/Library/Fonts/SFNS.ttf") {
        defs.font_data
            .insert("SFNS".to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
        proportional.push("SFNS".to_owned());
    }
    // The reference's mono stack (`ui-monospace, "SF Mono", "Cascadia Mono",
    // Menlo, …`) resolves to Menlo in Chrome on macOS — "SF Mono" is not an
    // installed family there. Menlo Regular's fixed advance is 1233/2048 em,
    // matching the reference's computed mono metrics exactly.
    if let Some(bytes) = sys("/System/Library/Fonts/Menlo.ttc") {
        // A .ttc collection; the Regular face is index 0.
        defs.font_data.insert(
            "Menlo".to_owned(),
            Arc::new(egui::FontData::from_blob(Arc::new(bytes), 0)),
        );
        monospace.push("Menlo".to_owned());
    }
    if let Some(bytes) = sys("/System/Library/Fonts/Apple Color Emoji.ttc") {
        // A .ttc collection; the emoji face is index 0.
        defs.font_data.insert(
            "AppleColorEmoji".to_owned(),
            Arc::new(egui::FontData::from_blob(Arc::new(bytes), 0)),
        );
        proportional.push("AppleColorEmoji".to_owned());
        monospace.push("AppleColorEmoji".to_owned());
    }

    // Only replace the families that resolved: on non-macOS systems the SF
    // files are absent and eframe's system font provider (fontique) resolves
    // the standard families instead.
    if !proportional.is_empty() {
        defs.families.insert(FontFamily::Proportional, proportional);
    }
    if !monospace.is_empty() {
        defs.families.insert(FontFamily::Monospace, monospace);
    }
    ctx.set_fonts(defs);

    // Unhinted grayscale AA, closest to the reference's
    // `-webkit-font-smoothing: antialiased` on macOS. Subpixel binning stays
    // on (Chrome on macOS subpixel-positions glyphs).
    ctx.all_styles_mut(|style| apply_text_options(&mut style.visuals));
}

/// The text rasterization options the reference matches: unhinted grayscale
/// AA (like `-webkit-font-smoothing: antialiased` on macOS) with subpixel
/// binning on (Chrome on macOS subpixel-positions glyphs).
///
/// These must stay identical everywhere visuals are (re)applied: egui
/// rebuilds the whole font atlas when the style's `text_options` change, and
/// galleys the app keeps across frames (the header title's segments) would be
/// left with stale atlas UVs, rendering as garbled fragments.
pub fn apply_text_options(visuals: &mut egui::Visuals) {
    visuals.text_options.font_hinting = false;
    visuals.text_options.subpixel_binning = true;
    // Pin the color transfer function as well: `Visuals::dark()` and
    // `Visuals::light()` default to different ones (`TwoCoverageMinusCoverageSq`
    // vs `Off`), and any change makes epaint rebuild the font atlas on the next
    // pass — galleys the app keeps across frames (the header title's segments)
    // would be left with stale atlas UVs and render as garbled fragments. The
    // reference (Chrome/Skia on macOS) antialiases text the same way in both
    // themes, and its compositing is linear coverage: measured against the
    // reference captures, raw linear coverage (`Off`) reproduces the
    // reference's glyph ink to within 0.7% on the proportional text (title,
    // button labels, section labels; both themes, both polarities, dsf 1 and
    // dsf 2) and within 2% on the Menlo message text, while egui's dark-mode
    // default `TwoCoverageMinusCoverageSq` overshoots to +4%…+15% ink and
    // `Gamma(0.85)` still overshoots the small text. The residual is
    // rasterizer coverage noise (ab_glyph vs the reference's rasterizer),
    // not a transfer difference.
    visuals.text_options.color_transfer_function = epaint::FontColorTransferFunction::Off;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `trak` tracking CoreText adds at the sizes the app uses, measured
    /// as Chrome's per-glyph advance minus egui's (identical font, same
    /// variation coordinates): exactly the `trak` default-track value.
    #[test]
    fn trak_tracking_matches_chrome_at_the_app_sizes() {
        let cases: &[(f32, f32)] = &[
            (11.25, 0.0494385),
            (12.0, 0.0),
            (12.375, -0.0271912),
            (13.125, -0.0849152),
            (13.875, -0.1405792),
            (14.0, -0.1503906),
            (14.8, -0.2167969),
            (15.0, -0.234375),
            (16.0, -0.3125),
            (22.5, -0.1812744),
            (24.0, 0.0703125),
            (30.0, 0.3955078),
            (32.0, 0.40625),
            (33.75, 0.3924179),
            (36.0, 0.3691406),
        ];
        for (size, want) in cases {
            let spec = TextSpec::new(*size, 300.0, size * 1.5, 0.0);
            assert!(
                (spec.trak_tracking() - want).abs() < 1e-4,
                "trak_tracking({size}) = {}, want {want}",
                spec.trak_tracking()
            );
        }
        // Menlo has no trak table.
        assert_eq!(CODE_DEFAULT_S.trak_tracking(), 0.0);
    }

    /// Galley widths must match the reference (Chrome) measurements. Chrome
    /// adds CSS letter-spacing and CoreText's `trak` tracking after the last
    /// glyph as well; egui adds letter-spacing only between glyphs, so the
    /// comparison adds the trailing unit back.
    #[test]
    fn galley_widths_match_the_chrome_reference() {
        let ctx = Context::default();
        install_fonts(&ctx);
        let mut out = ctx.run_pass(Default::default(), |_| {});
        out.textures_delta.clear();

        // (name, spec, text, Chrome-measured width in px)
        let cases: &[(&str, TextSpec, &str, f32)] = &[
            ("title", TITLE_M, "SpeakEasy", 159.9688),
            ("heading", DISPLAY_STRONG_S, "Message approval request", 344.21875),
            ("conv_title", BODY_STRONG_S, "Ship gateway v0.3", 116.546875),
            ("approve", BUTTON_LABEL, "Approve", 50.703125),
            ("submit", BUTTON_LABEL, "Submit feedback", 102.671875),
            ("give", BUTTON_LABEL, "Give feedback", 86.296875),
            ("deny", BUTTON_LABEL, "Deny", 30.90625),
            ("tag_agent", LABEL_DEFAULT_S, "claude-code", 73.046875),
            ("subline", BODY_DEFAULT_XS, "Request demo-0001 \u{b7} received 12:00:00\u{202f}PM", 231.46875),
            ("reason_head", BODY_DEFAULT_S, "The team asked for deploy notifications; the", 265.234375),
            ("ta_label", LABEL_DEFAULT_M, "Feedback for the requesting agent", 219.109375),
            ("label_msg", LABEL_DEFAULT_XS, "Proposed message (exact text)", 164.8125),
            ("label_dest", LABEL_DEFAULT_XS, "Destination (Slack)", 100.1875),
            ("label_conv", LABEL_DEFAULT_XS, "Originating agent conversation", 162.796875),
            ("empty_title", BODY_STRONG_M, "Waiting for agent requests", 189.453125),
            // the display sizes at the ≥769px / ≥1025px breakpoints (Once UI
            // steps `--font-size-display-m/-s` up with the viewport width)
            (
                "title_r16",
                TextSpec::new(44.0, 600.0, 48.0, -1.32),
                "SpeakEasy",
                207.2344,
            ),
            (
                "heading_r16",
                TextSpec::new(36.0, 600.0, 40.0, -0.72),
                "Message approval request",
                410.53125,
            ),
            (
                "title_r16xl",
                TextSpec::new(48.0, 600.0, 52.0, -1.44),
                "SpeakEasy",
                225.6563,
            ),
            (
                "heading_r16xl",
                TextSpec::new(40.0, 600.0, 48.0, -0.8),
                "Message approval request",
                455.203125,
            ),
        ];
        for (name, spec, text, chrome) in cases {
            let galley = layout(&ctx, spec.job(text, Color32::WHITE, f32::INFINITY, false));
            let got = galley.rect.width() + spec.letter_spacing + spec.trak_tracking();
            assert!(
                (got - chrome).abs() <= 0.15,
                "{name}: width {got:.4} (galley {:.4} + trailing {:.4}), chrome {chrome:.4}",
                galley.rect.width(),
                spec.letter_spacing + spec.trak_tracking()
            );
        }

        // Menlo (the reference's mono resolution): no trak, and Chrome's
        // shaper differs from egui's by at most ~0.003px per glyph.
        let msg = "Heads-up: gateway v0.3 finished deploying \u{2014} 12/12 checks green. Rolling out to the remaining workspaces now.";
        let link = "https://app.superset.sh/workspaces/speakeasy/tasks/deploy-gateway-v03";
        let mono_cases: &[(&str, TextSpec, &str, f32)] = &[
            ("mono_msg", CODE_DEFAULT_S, msg, 804.328125),
            ("mono_link", CODE_DEFAULT_XS, link, 467.34375),
            ("mono_n100_12375", CODE_DEFAULT_S, &"n".repeat(100), 744.75),
            ("mono_n100_1125", CODE_DEFAULT_XS, &"n".repeat(100), 677.3125),
        ];
        for (name, spec, text, chrome) in mono_cases {
            let galley = layout(&ctx, spec.job(text, Color32::WHITE, f32::INFINITY, false));
            assert!(
                (galley.rect.width() - chrome).abs() <= 0.4,
                "{name}: width {:.4}, chrome {chrome:.4}",
                galley.rect.width()
            );
        }
    }

    /// Regression: re-applying visuals (as `ApprovalApp::apply_visuals` does on
    /// init and theme changes) must not change the text rasterization options.
    /// egui rebuilds the whole font atlas when the style's `text_options`
    /// change, which orphans the atlas UVs of galleys the app keeps across
    /// frames — the header title's segments rendered as tiny garbled text.
    #[test]
    fn reapplying_visuals_keeps_the_font_atlas_stable() {
        let ctx = Context::default();
        install_fonts(&ctx);
        // The fonts (and their atlas) are created by the first pass.
        let mut out = ctx.run_pass(Default::default(), |_| {});
        out.textures_delta.clear();
        let options_before = ctx.style_of(egui::Theme::Dark).visuals.text_options;

        // Lay out a galley and checksum its first glyph's atlas region.
        let galley = layout(&ctx, TITLE_M.job("Zg", Color32::WHITE, f32::INFINITY, false));
        let checksum = |ctx: &Context| -> u64 {
            ctx.fonts(|f| {
                let atlas = f.image();
                let g = &galley.rows[0].glyphs[0];
                let (x0, y0) = (g.uv_rect.min[0] as usize, g.uv_rect.min[1] as usize);
                let (w, h) = (
                    (g.uv_rect.max[0] - g.uv_rect.min[0]) as usize,
                    (g.uv_rect.max[1] - g.uv_rect.min[1]) as usize,
                );
                let mut sum = 0u64;
                for yy in 0..h.min(atlas.height()) {
                    for xx in 0..w.min(atlas.width()) {
                        let p = atlas.pixels[(y0 + yy) * atlas.width() + (x0 + xx)];
                        sum = sum.wrapping_add(p.a() as u64);
                    }
                }
                sum
            })
        };
        let before = checksum(&ctx);
        assert!(before > 0, "glyph bitmap missing from the atlas");

        // Re-apply visuals the way `apply_visuals` does, then run another pass
        // (egui compares the text options and rebuilds the atlas in begin_pass).
        let mut visuals = egui::Visuals::dark();
        apply_text_options(&mut visuals);
        ctx.set_visuals(visuals);
        let options_after = ctx.style_of(egui::Theme::Dark).visuals.text_options;
        assert_eq!(
            options_before.font_hinting, options_after.font_hinting,
            "font_hinting drifted when visuals were re-applied"
        );
        assert_eq!(
            options_before.subpixel_binning, options_after.subpixel_binning,
            "subpixel_binning drifted when visuals were re-applied"
        );
        let mut out = ctx.run_pass(Default::default(), |_| {});
        out.textures_delta.clear();

        let after = checksum(&ctx);
        assert_eq!(
            before, after,
            "font atlas was rebuilt after re-applying visuals; long-lived galleys would keep stale UVs"
        );
    }

    /// The reference's (Chrome's) first-line baseline offsets, measured in the
    /// running reference app with a zero-height `vertical-align: baseline`
    /// marker at the start of every text block, cross-checked against a
    /// synthetic grid of every style at both rem roots (15px and 16px) and at
    /// device scales 1 and 2: Chrome rounds the font's CoreText ascent and
    /// descent to whole pixels and places the baseline at
    /// `a + floor((line_height - (a + d)) / 2)` below the line-box top —
    /// always a whole number of px.
    #[test]
    fn chrome_baseline_offsets_match_the_reference() {
        let s16 = 16.0 / 15.0;
        let cases: &[(TextSpec, f32)] = &[
            // root 15
            (TITLE_M, 31.0),
            (DISPLAY_STRONG_S, 30.0),
            (BODY_DEFAULT_S, 13.0),
            (BODY_STRONG_S, 13.0),
            (BODY_DEFAULT_XS, 12.0),
            (BODY_STRONG_M, 17.0),
            (LABEL_DEFAULT_XS, 11.0),
            (LABEL_DEFAULT_S, 12.0),
            (LABEL_DEFAULT_M, 14.0),
            (BUTTON_LABEL, 12.0),
            (CODE_DEFAULT_S, 11.0),
            (CODE_DEFAULT_XS, 10.0),
            (FIELD_TEXT, 17.0),
            // the textarea's floating label (`label_m` scaled 0.75)
            (TextSpec::new(10.40625, 300.0, 14.0625, 0.0), 11.0),
            // root 16, 769–1024px: the display tokens step up (Once UI
            // `--font-size-display-m` 2.75rem / `-s` 2.25rem at a 16px root)
            (TextSpec::new(44.0, 600.0, 48.0, -1.32), 41.0),
            (TextSpec::new(36.0, 600.0, 40.0, -0.72), 33.0),
            // root 16, ≥1025px: the display tokens step up again
            // (`--font-size-display-m` 3rem / `-s` 2.5rem)
            (TextSpec::new(48.0, 600.0, 52.0, -1.44), 44.0),
            (TextSpec::new(40.0, 600.0, 48.0, -0.8), 39.0),
            // root 16 (everything else scaled by 16/15)
            (BODY_DEFAULT_S.scaled(s16), 14.0),
            (BODY_STRONG_S.scaled(s16), 14.0),
            (BODY_DEFAULT_XS.scaled(s16), 12.0),
            (BODY_STRONG_M.scaled(s16), 18.0),
            (LABEL_DEFAULT_XS.scaled(s16), 11.0),
            (LABEL_DEFAULT_S.scaled(s16), 13.0),
            (LABEL_DEFAULT_M.scaled(s16), 15.0),
            (BUTTON_LABEL.scaled(s16), 13.0),
            (CODE_DEFAULT_S.scaled(s16), 12.0),
            (CODE_DEFAULT_XS.scaled(s16), 11.0),
            (FIELD_TEXT.scaled(s16), 18.0),
            // the textarea's floating label at root 16
            (
                TextSpec {
                    size: LABEL_DEFAULT_M.scaled(s16).size * 0.75,
                    weight: 300.0,
                    line_height: LABEL_DEFAULT_M.scaled(s16).line_height * 0.75,
                    letter_spacing: 0.0,
                    mono: false,
                },
                12.0,
            ),
        ];
        for (spec, want) in cases {
            let got = spec.chrome_baseline_offset();
            assert_eq!(
                got, *want,
                "baseline offset for {}px lh {} = {got}, want {want}",
                spec.size, spec.line_height
            );
        }
    }

    /// Painting a galley at `pos + (0, baseline_correction(galley))` must put
    /// its first-line baseline exactly on the reference's offset, at both
    /// device scales the app runs at (the correction is derived from the
    /// galley's own layout job, so this also guards the job → style
    /// reconstruction).
    #[test]
    fn baseline_correction_places_galleys_on_the_reference() {
        let s16 = 16.0 / 15.0;
        let specs: &[TextSpec] = &[
            TITLE_M,
            DISPLAY_STRONG_S,
            BODY_DEFAULT_S,
            BODY_STRONG_S,
            BODY_DEFAULT_XS,
            BODY_STRONG_M,
            LABEL_DEFAULT_XS,
            LABEL_DEFAULT_S,
            LABEL_DEFAULT_M,
            BUTTON_LABEL,
            CODE_DEFAULT_S,
            CODE_DEFAULT_XS,
            FIELD_TEXT,
            TextSpec::new(10.40625, 300.0, 14.0625, 0.0),
            TextSpec::new(44.0, 600.0, 48.0, -1.32),
            TextSpec::new(36.0, 600.0, 40.0, -0.72),
            TextSpec::new(48.0, 600.0, 52.0, -1.44),
            TextSpec::new(40.0, 600.0, 48.0, -0.8),
            BODY_DEFAULT_S.scaled(s16),
            BODY_STRONG_S.scaled(s16),
            BODY_DEFAULT_XS.scaled(s16),
            BODY_STRONG_M.scaled(s16),
            LABEL_DEFAULT_XS.scaled(s16),
            LABEL_DEFAULT_S.scaled(s16),
            LABEL_DEFAULT_M.scaled(s16),
            BUTTON_LABEL.scaled(s16),
            CODE_DEFAULT_S.scaled(s16),
            CODE_DEFAULT_XS.scaled(s16),
            FIELD_TEXT.scaled(s16),
        ];
        for ppp in [1.0f32, 2.0] {
            let ctx = Context::default();
            ctx.set_pixels_per_point(ppp);
            install_fonts(&ctx);
            let mut out = ctx.run_pass(Default::default(), |_| {});
            out.textures_delta.clear();
            // Line-box tops the app actually uses (logical px), including the
            // fractional ones: the correction must snap the rendered baseline
            // to whole CSS pixels at every one of them.
            let line_tops = [0.0f32, 0.375, 114.5, 222.375, 602.375];
            for spec in specs {
                let galley = layout(&ctx, spec.job("Hxgjn", Color32::WHITE, f32::INFINITY, false));
                for &line_top in &line_tops {
                    let dy = baseline_correction(&galley, line_top);
                    let painted = line_top + galley_baseline(&galley) + dy;
                    let want = (line_top + spec.chrome_baseline_offset()).round();
                    assert!(
                        (painted - want).abs() < 1e-4,
                        "ppp {ppp} {}px @ top {line_top}: painted baseline {painted:.4}, want {want}",
                        spec.size
                    );
                    // The correction only compensates egui's metric rounding,
                    // device-pixel snapping, and the reference's CSS-px
                    // baseline snap (≤ 0.5); it must stay small.
                    assert!(
                        dy.abs() <= 1.5,
                        "ppp {ppp} {}px @ top {line_top}: correction {dy} too large",
                        spec.size
                    );
                }
            }

            // Multi-line galleys: Chrome snaps every line's rendered
            // baseline to whole CSS px (`round(line_top + k*line_height +
            // target)`), while egui snaps the row pitch to whole device
            // pixels (16.875 → 17.0 at 2×). `snapped_galley` must place every
            // row exactly — at the line tops the app uses and across
            // fractional tops.
            let spec = BODY_DEFAULT_S;
            let galley = layout(
                &ctx,
                spec.job(
                    "Hxgjn Hxgjn Hxgjn Hxgjn Hxgjn Hxgjn Hxgjn Hxgjn Hxgjn Hxgjn Hxgjn Hxgjn Hxgjn Hxgjn Hxgjn Hxgjn Hxgjn Hxgjn",
                    Color32::WHITE,
                    200.0,
                    false,
                ),
            );
            assert!(galley.rows.len() >= 3);
            let target = spec.chrome_baseline_offset();
            let check = |line_top: f32| {
                let snapped = snapped_galley(&galley, line_top);
                // `paint_galley` paints the snapped galley at the line top,
                // or the original at the line top plus the first-line
                // correction when no per-row adjustment is needed.
                let base = if Arc::ptr_eq(&snapped, &galley) {
                    line_top + baseline_correction(&galley, line_top)
                } else {
                    line_top
                };
                for (k, row) in snapped.rows.iter().enumerate() {
                    let painted = base + row.pos.y + row.glyphs[0].pos.y;
                    let want = (line_top + k as f32 * spec.line_height + target).round();
                    assert!(
                        (painted - want).abs() < 1e-4,
                        "ppp {ppp} @ top {line_top}: row {k} baseline {painted:.4}, want {want}"
                    );
                }
                if let Some(shifts) = row_baseline_shifts(&galley, line_top) {
                    // The first row's shift is exactly the first-line correction.
                    let dy = baseline_correction(&galley, line_top);
                    assert!((shifts[0] - dy).abs() < 1e-4);
                }
            };
            for &line_top in &[386.625f32, 503.875, 222.375] {
                check(line_top);
            }
            let mut f = 0.0f32;
            while f < 1.0 {
                check(222.375 + f);
                f += 1.0 / 16.0;
            }

            // Galleys that need no per-row adjustment take the cheap path: a
            // single row, or an integral line-height whose snapped pitch is
            // exact (the selectable message text, 15px).
            let single = layout(&ctx, spec.job("Hxgjn", Color32::WHITE, f32::INFINITY, false));
            assert!(row_baseline_shifts(&single, 222.375).is_none());
            let code = CODE_DEFAULT_S;
            let code_galley = layout(
                &ctx,
                code.job(
                    "Hxgjn Hxgjn Hxgjn Hxgjn Hxgjn Hxgjn Hxgjn Hxgjn Hxgjn Hxgjn",
                    Color32::WHITE,
                    200.0,
                    false,
                ),
            );
            assert!(code_galley.rows.len() >= 3);
            assert!(row_baseline_shifts(&code_galley, 222.375).is_none());
            // A fractional line-height needs per-row snapping even when the
            // device-pixel pitch is exact (22.5 × 2 = 45): Chrome snaps each
            // rendered baseline individually.
            let field = FIELD_TEXT;
            let field_galley = layout(
                &ctx,
                field.job(
                    "Hxgjn Hxgjn Hxgjn Hxgjn Hxgjn Hxgjn Hxgjn Hxgjn Hxgjn Hxgjn",
                    Color32::WHITE,
                    200.0,
                    false,
                ),
            );
            assert!(field_galley.rows.len() >= 3);
            assert!(row_baseline_shifts(&field_galley, 614.83).is_some());
        }
    }

    /// CSS `overflow-wrap: anywhere` must reproduce Chrome's exact line
    /// breaks (measured on the reference at 720px, root 15): a word that
    /// does not fit the remaining width moves to the next line whole, and
    /// only a word wider than a full line is broken, at the last character
    /// that fits. egui's native `break_anywhere` fills the remaining width
    /// with the start of the word instead — the long-state message wrapped
    /// one line short and shifted the whole card.
    #[test]
    fn css_anywhere_wrapping_matches_chrome() {
        let ctx = Context::default();
        install_fonts(&ctx);
        let mut out = ctx.run_pass(Default::default(), |_| {});
        out.textures_delta.clear();

        let rows_of = |galley: &Galley| -> Vec<String> {
            galley.rows.iter().map(|r| r.row.text()).collect()
        };

        // The long-seed proposed message at the reference's wrap width
        // (628 − 2×(1 + 11.25) = 603.5): Chrome renders 11 lines; the
        // 108-char token moves to its own line and is broken after its
        // 81st char (the last that fits 603.5px), not after the `.` at 37.
        let msg = "Deploy of release 4.2.1 to production finished successfully.\n\n\u{2022} api: 12/12 checks green \u{2705}\n\u{2022} web: 12/12 checks green \u{2705}\n\u{2022} db migration 042 applied\n\nA very long unbreakable token: eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U\n\nRollout continues to the remaining workspaces.";
        let galley = galley_anywhere(&ctx, CODE_DEFAULT_S, msg, Color32::WHITE, 603.5);
        let rows = rows_of(&galley);
        assert_eq!(
            rows,
            vec![
                "Deploy of release 4.2.1 to production finished successfully.".to_string(),
                String::new(),
                "\u{2022} api: 12/12 checks green \u{2705}".to_string(),
                "\u{2022} web: 12/12 checks green \u{2705}".to_string(),
                "\u{2022} db migration 042 applied".to_string(),
                String::new(),
                "A very long unbreakable token: ".to_string(),
                "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmN".to_string(),
                "Hl0w5N_XgL0n3I9PlFUP0THsR8U".to_string(),
                String::new(),
                "Rollout continues to the remaining workspaces.".to_string(),
            ]
        );
        assert_eq!(rows[7].chars().count(), 81);
        let hard: Vec<bool> = galley.rows.iter().map(|r| r.ends_with_newline).collect();
        assert_eq!(
            hard,
            vec![true, true, true, true, true, true, false, false, true, true, false]
        );
        // The assembled galley keeps the full text: selection cursors map
        // through the rows exactly like a natively-wrapped galley.
        assert_eq!(galley.text(), msg);
        let total: usize = galley
            .rows
            .iter()
            .map(|r| r.glyphs.len() + r.ends_with_newline as usize)
            .sum();
        assert_eq!(total, msg.chars().count());

        // The long-seed conversation link at the reference's conversation
        // content width (596): Chrome breaks after the last hyphen that
        // fits (77 chars) — the period and the 88th char are not break
        // points.
        let link = "https://app.superset.sh/workspaces/speakeasy/tasks/release-4-2-1-prod-deploy-postmortem-summary-042";
        let galley = galley_anywhere(&ctx, CODE_DEFAULT_XS, link, Color32::WHITE, 596.0);
        assert_eq!(
            rows_of(&galley),
            vec![
                "https://app.superset.sh/workspaces/speakeasy/tasks/release-4-2-1-prod-deploy-".to_string(),
                "postmortem-summary-042".to_string(),
            ]
        );

        // The single-seed message wraps after the last space that fits.
        let msg1 = "Heads-up: gateway v0.3 finished deploying \u{2014} 12/12 checks green. Rolling out to the remaining workspaces now.";
        let galley = galley_anywhere(&ctx, CODE_DEFAULT_S, msg1, Color32::WHITE, 603.5);
        assert_eq!(
            rows_of(&galley),
            vec![
                "Heads-up: gateway v0.3 finished deploying \u{2014} 12/12 checks green. Rolling out to ".to_string(),
                "the remaining workspaces now.".to_string(),
            ]
        );

        // A text that fits needs no wrapping; empty lines keep their row.
        let galley = galley_anywhere(&ctx, CODE_DEFAULT_S, "one line", Color32::WHITE, 603.5);
        assert_eq!(rows_of(&galley), vec!["one line".to_string()]);
        let galley = galley_anywhere(&ctx, CODE_DEFAULT_S, "a\n\nb", Color32::WHITE, 603.5);
        assert_eq!(rows_of(&galley), vec!["a".to_string(), String::new(), "b".to_string()]);
        let hard: Vec<bool> = galley.rows.iter().map(|r| r.ends_with_newline).collect();
        assert_eq!(hard, vec![true, true, false]);
    }

    /// The assembled anywhere-galley must place every row's baseline where
    /// the reference renders it (per-row snapping works on it like on a
    /// natively-wrapped galley), and its block height must be the CSS line
    /// count × line-height.
    #[test]
    fn css_anywhere_galley_snaps_like_a_native_galley() {
        for ppp in [1.0f32, 2.0] {
            let ctx = Context::default();
            ctx.set_pixels_per_point(ppp);
            install_fonts(&ctx);
            let mut out = ctx.run_pass(Default::default(), |_| {});
            out.textures_delta.clear();
            let msg = "A very long unbreakable token: eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U\n\nRollout continues to the remaining workspaces.";
            let galley = galley_anywhere(&ctx, CODE_DEFAULT_S, msg, Color32::WHITE, 603.5);
            assert!(galley.rows.len() >= 3);
            assert_eq!(
                galley.rows.len() as f32 * CODE_DEFAULT_S.line_height,
                crate::ui::components::block_height(&galley, CODE_DEFAULT_S)
            );
            let target = CODE_DEFAULT_S.chrome_baseline_offset();
            for &line_top in &[222.375f32, 602.375] {
                let snapped = snapped_galley(&galley, line_top);
                let base = if Arc::ptr_eq(&snapped, &galley) {
                    line_top + baseline_correction(&galley, line_top)
                } else {
                    line_top
                };
                for (k, row) in snapped.rows.iter().enumerate() {
                    // Empty rows (from `\n\n`) paint nothing; the snapping
                    // shifts them like the first row (see row_baseline_shifts).
                    if row.glyphs.is_empty() {
                        continue;
                    }
                    let painted = base + row.pos.y + row.glyphs[0].pos.y;
                    let want = (line_top + k as f32 * CODE_DEFAULT_S.line_height + target).round();
                    assert!(
                        (painted - want).abs() < 1e-4,
                        "ppp {ppp} @ top {line_top}: row {k} baseline {painted:.4}, want {want}"
                    );
                }
            }
        }
    }
}
