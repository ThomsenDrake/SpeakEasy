//! Frame painting: layout computation and rendering of the approval window.
//!
//! The window is fixed chrome (header + footer) with a single scrolling main
//! region holding one request card (or the empty state). Every geometry
//! value is the *resolved* geometry of the reference UI at its responsive
//! rem root (Once UI: root 15px at viewport widths ≤768px, 16px above), so
//! all rem-based constants are scaled per frame through [`Metrics`]; absolute
//! values (1px borders, the literal lucide icon sizes) are not scaled.
//!
//! Layout is computed first ([`ApprovalApp::compute_layout`]) and painted
//! from the resulting rects, so the exact capture-verified positions (card
//! at (22.5, 91, 675, …), content x=46/cw=628 at 720px, header 68.5px, …) are
//! produced by one code path that is also exercised by the unit tests in
//! `paint/tests.rs`.

use std::sync::Arc;

use eframe::egui::{Color32, Context, Id, Pos2, Rect, Sense, Ui, pos2, vec2};
use epaint::text::Galley;

use crate::core::{Decision, MessageRequest};
use crate::ui::anim::SegmentState;
use crate::ui::components::{
    self, AppEvent, ButtonVariant, Ctx, FocusId, Justify, RowAlign, TagScheme, Tween,
};
use crate::ui::icons::Icon;
use crate::ui::shaders::{self, TitleBlur};
use crate::ui::sys;
use crate::ui::theme::Metrics;

use super::{
    ACTIONS_GAP, ApprovalApp, BTN_APPROVE, BTN_DENY, BTN_GIVE_FEEDBACK, BTN_SUBMIT_FEEDBACK,
    CARD_GAP, CARD_PAD, CONV_PANEL_PAD, COOLDOWN_PREFIX, EMPTY_GAP, EMPTY_PAD, EMPTY_SUB,
    EMPTY_TITLE, FOOTER_GAP, FOOTER_PAD_X, FOOTER_PAD_Y, FOOTER_TEXT, HEADING, HEADER_PAD_X,
    HEADER_PAD_Y, HINT_CONVERSATION, HINT_DESTINATION, LABEL_CONVERSATION, LABEL_DESTINATION,
    LABEL_MESSAGE, LABEL_REASON, MAIN_PAD, MSG_PANEL_PAD, SCROLLBAR_MIN_THUMB,
    SCROLLBAR_THUMB_ALPHA, SCROLLBAR_THUMB_ALPHA_HOVER, SECTION_GAP, SUBMIT_ERROR, TEXTAREA_LABEL,
    TEXTAREA_PLACEHOLDER, TitleSegPaint, button_id,
};

#[cfg(test)]
mod tests;

/// Scroll a point up by the current scroll offset.
fn py(scroll: f32, p: Pos2) -> Pos2 {
    pos2(p.x, p.y - scroll)
}

/// Scroll a rect up by the current scroll offset.
fn sy(scroll: f32, r: Rect) -> Rect {
    Rect::from_min_size(py(scroll, r.min), r.size())
}

/// A laid-out text galley plus its absolute (unscrolled) position.
struct TextBlock {
    galley: Arc<Galley>,
    pos: Pos2,
}

/// A selectable text run: its widget id (per-request, so selection state
/// resets with the request), position, wrap width and laid-out height.
struct SelText {
    id: Id,
    pos: Pos2,
    wrap: f32,
    /// CSS block height (rows × line-height); asserted by the layout tests.
    #[allow(dead_code)]
    h: f32,
}

/// Pre-computed geometry of the request card (all rects/positions are
/// absolute and unscrolled; painting subtracts the scroll offset).
struct CardLayout {
    /// The card's border box (height = content_h + 2 × (1px border + padding)).
    card: Rect,
    // Content geometry (x, width, subline copy, content height): part of the
    // layout contract asserted by the tests.
    #[allow(dead_code)]
    content_x: f32,
    #[allow(dead_code)]
    cw: f32,
    heading: TextBlock,
    subline: TextBlock,
    #[allow(dead_code)]
    subline_text: String,
    agent_tag: (Rect, String),
    label_msg: TextBlock,
    msg_panel: Rect,
    msg_text: SelText,
    label_dest: TextBlock,
    dest_hash: Pos2,
    info_tag: (Rect, String),
    raw_tag: Option<(Rect, String)>,
    thread_tag: Option<(Rect, String)>,
    hint_dest: TextBlock,
    label_reason: TextBlock,
    reason: SelText,
    label_conv: TextBlock,
    conv_panel: Rect,
    conv_icon: Pos2,
    conv_title: TextBlock,
    conv_link: SelText,
    conv_summary: SelText,
    hint_conv: TextBlock,
    ta_box: Option<Rect>,
    submit_btn: Option<Rect>,
    error_text: Option<TextBlock>,
    btn_give: Rect,
    btn_deny: Rect,
    btn_approve: Rect,
    #[allow(dead_code)]
    content_h: f32,
}

/// Pre-computed geometry of the empty state.
struct EmptyLayout {
    card: Rect,
    icon: Pos2,
    icon_size: f32,
    /// Centered galleys are painted at their *center* x (egui centers each
    /// row on the galley's origin).
    title: TextBlock,
    sub: TextBlock,
    #[allow(dead_code)]
    content_h: f32,
}

/// The footer's active-cooldown status row.
struct CooldownRow {
    row_top: f32,
    row_h: f32,
    dot_x: f32,
    text: TextBlock,
    #[allow(dead_code)]
    text_string: String,
}

/// Full-window layout for one frame.
struct Layout {
    screen: Rect,
    header: Rect,
    main: Rect,
    footer: Rect,
    slack_icon: Pos2,
    title_pos: Pos2,
    badge: Option<(Rect, String)>,
    /// The main region's scrollHeight (card + 2 × main padding).
    content_height: f32,
    max_scroll: f32,
    card_layout: Option<CardLayout>,
    empty: Option<EmptyLayout>,
    footer_text: TextBlock,
    cooldown_row: Option<CooldownRow>,
}

/// GPU texture + ink bounds for one title segment (the blurred reveal).
pub(crate) struct TitleTex {
    shared: Arc<shaders::TitleBlurShared>,
    /// The ink bounds' min in device pixels, galley-relative.
    ink_min: [f32; 2],
    /// The texture's padding in device pixels.
    pad: u32,
    /// The padded texture's size in device pixels.
    size: [f32; 2],
}

impl Clone for TitleTex {
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
            ink_min: self.ink_min,
            pad: self.pad,
            size: self.size,
        }
    }
}

/// The height of a size-m Tag (label-s line box + space-2 padding + borders).
fn tag_h_m(m: &Metrics) -> f32 {
    m.label_s.line_height + 2.0 * m.sp2 + 2.0
}

/// The height of a size-s Tag (label-s line box + absolute 1px padding + borders).
fn tag_h_s(m: &Metrics) -> f32 {
    m.label_s.line_height + 2.0 * m.sp1 + 2.0
}

impl ApprovalApp {
    /// Compute the full-window layout for the current state.
    fn compute_layout(&self, ctx: &Context, m: &Metrics, now: f64) -> Layout {
        let screen = ctx.input(|i| i.viewport_rect());
        let pad_x = m.scale(HEADER_PAD_X);
        let pad_y = m.scale(HEADER_PAD_Y);
        let main_pad = m.scale(MAIN_PAD);

        // ---- Header -------------------------------------------------------
        // Row (between/center, wrap): [slack icon + title] … [badge].
        let title_lh = m.title.line_height;
        let slack_side = 20.0; // literal lucide `size={20}` — absolute
        let badge = if self.queue.is_empty() {
            None
        } else {
            let count = if self.reduce_motion {
                self.badge.display_reduced()
            } else {
                self.badge.display(now)
            };
            let label = format!("{count} waiting");
            let w = components::tag_width(ctx, m, &label, None, self.pal.warning_on_bg_medium);
            let h = tag_h_m(m);
            Some((
                Rect::from_min_size(
                    pos2(screen.right() - pad_x - w, pad_y + (title_lh - h) / 2.0),
                    vec2(w, h),
                ),
                label,
            ))
        };
        let row_h = title_lh
            .max(slack_side)
            .max(badge.as_ref().map(|(r, _)| r.height()).unwrap_or(0.0));
        let header_h = pad_y * 2.0 + row_h + 1.0; // + 1px border-bottom
        let header = Rect::from_min_size(screen.min, vec2(screen.width(), header_h));
        let slack_icon = pos2(pad_x, pad_y + (row_h - slack_side) / 2.0);
        let title_pos = pos2(
            pad_x + slack_side + m.sp8,
            pad_y + (row_h - title_lh) / 2.0,
        );

        // ---- Footer -------------------------------------------------------
        let footer_pad_x = m.scale(FOOTER_PAD_X);
        let footer_pad_y = m.scale(FOOTER_PAD_Y);
        let footer_text_w = screen.width() - 2.0 * footer_pad_x;
        let footer_galley = components::galley(
            ctx,
            m.body_xs,
            FOOTER_TEXT,
            self.pal.text_medium,
            footer_text_w,
            false,
        );
        let footer_text_h = components::block_height(&footer_galley, m.body_xs);
        let mut footer_content_h = footer_text_h;
        let mut cooldown_row = None;
        if !self.cooldowns.is_empty() {
            let dot = m.sp4;
            let gap = m.sp8;
            let text_string = format!(
                "{}{}",
                COOLDOWN_PREFIX,
                self.cooldowns
                    .iter()
                    .map(|(dest, _, secs)| format!("{dest} ({} min left)", secs.div_ceil(60)))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            let galley = components::galley(
                ctx,
                m.body_xs,
                &text_string,
                self.pal.warning_on_bg_medium,
                footer_text_w - dot - gap,
                false,
            );
            let row_h = m.body_xs.line_height;
            footer_content_h += m.scale(FOOTER_GAP) + row_h;
            cooldown_row = Some((row_h, galley, text_string));
        }
        // + 1px border-top.
        let footer_h = 1.0 + footer_pad_y * 2.0 + footer_content_h;
        let footer = Rect::from_min_size(
            pos2(screen.min.x, screen.max.y - footer_h),
            vec2(screen.width(), footer_h),
        );
        let footer_text_pos = pos2(
            footer.min.x + footer_pad_x,
            footer.min.y + 1.0 + footer_pad_y,
        );
        let cooldown_row = cooldown_row.map(|(row_h, galley, text_string)| {
            let row_top = footer_text_pos.y + footer_text_h + m.scale(FOOTER_GAP);
            let dot_x = footer.min.x + footer_pad_x;
            CooldownRow {
                row_top,
                row_h,
                dot_x,
                text: TextBlock {
                    galley,
                    pos: pos2(dot_x + m.sp4 + m.sp8, row_top),
                },
                text_string,
            }
        });
        let footer_text = TextBlock {
            galley: footer_galley,
            pos: footer_text_pos,
        };

        // ---- Main / card --------------------------------------------------
        let main = Rect::from_min_max(
            pos2(screen.min.x, header.max.y),
            pos2(screen.max.x, footer.min.y),
        );
        let card_w = (main.width() - 2.0 * main_pad).min(m.card_max_w);
        let card_x = main.min.x + (main.width() - card_w) / 2.0;
        let card_top = main.min.y + main_pad;

        let (card_layout, empty, card) = match self.queue.front() {
            Some(req) => {
                let cl = compute_card(self, ctx, m, req, card_x, card_top, card_w);
                let card = cl.card;
                (Some(cl), None, card)
            }
            None => {
                let el = compute_empty(self, ctx, m, card_x, card_top, card_w);
                let card = el.card;
                (None, Some(el), card)
            }
        };
        let content_height = card.height() + 2.0 * main_pad;
        let max_scroll = (content_height - main.height()).max(0.0);

        Layout {
            screen,
            header,
            main,
            footer,
            slack_icon,
            title_pos,
            badge,
            content_height,
            max_scroll,
            card_layout,
            empty,
            footer_text,
            cooldown_row,
        }
    }

    /// Paint one frame.
    pub(crate) fn paint_frame(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let m = Metrics::new(ctx.input(|i| i.viewport_rect().width()));
        let now = self.frame_now;
        let ppp = ctx.pixels_per_point();
        let layout = self.compute_layout(&ctx, &m, now);

        // Clamp and store the scroll state (consumed by `logic`).
        let scroll_offset = self.scroll_offset.clamp(0.0, layout.max_scroll);
        self.scroll_offset = scroll_offset;
        self.main_rect = layout.main;
        self.content_height = layout.content_height;
        self.max_scroll = layout.max_scroll;

        // Take the mutable per-frame state out of self so painting can borrow
        // the context freely.
        let mut events = std::mem::take(&mut self.events);
        let mut focus = std::mem::take(&mut self.focus);
        // The tab order is the DOM order of the controls painted (and enabled)
        // this frame — e.g. the feedback textarea and submit button only exist
        // while the feedback panel is open, and the disabled submit button is
        // skipped, like the browser.
        focus.order.clear();
        let mut fb_text = std::mem::take(&mut self.feedback_text);
        let mut hover_until = self.hover_repaint_until;
        let pal = self.pal.clone();
        let reduce_motion = self.reduce_motion;

        let press_consumed = {
            let mut c = Ctx {
                ui,
                pal: &pal,
                m: &m,
                now,
                reduce_motion,
                events: &mut events,
                focus: &mut focus,
                press_consumed: false,
            };

            // ---- Header ---------------------------------------------------
            components::icon_at(&c, Icon::Slack, layout.slack_icon, 20.0, pal.brand_on_bg_strong);
            let title_paint = self.prepare_title(&c, &layout);
            let container = if reduce_motion {
                1.0
            } else {
                self.title_fx.container_opacity(now)
            };
            let [tr, tg, tb, _] = pal.text_strong.to_array();
            for seg in &title_paint {
                match (&seg.state, &seg.tex) {
                    (Some(st), Some(tex)) => {
                        let alpha = st.opacity * container;
                        // The texture blits the galley's glyphs at their
                        // uncorrected positions; shift the quad so the
                        // blurred text lands where the revealed text does.
                        let dy = crate::ui::text::baseline_correction(
                            &seg.galley,
                            layout.title_pos.y,
                        );
                        let rect = Rect::from_min_size(
                            pos2(
                                seg.x + (tex.ink_min[0] - tex.pad as f32) / ppp,
                                layout.title_pos.y
                                    + dy
                                    + st.y_offset
                                    + (tex.ink_min[1] - tex.pad as f32) / ppp,
                            ),
                            vec2(tex.size[0] / ppp, tex.size[1] / ppp),
                        );
                        let sigma = if reduce_motion { 0.0 } else { st.blur * ppp };
                        c.painter().add(
                            TitleBlur::new(
                                Arc::clone(&tex.shared),
                                rect,
                                [
                                    tr as f32 / 255.0,
                                    tg as f32 / 255.0,
                                    tb as f32 / 255.0,
                                    alpha,
                                ],
                                sigma,
                            )
                            .shape(),
                        );
                    }
                    // Fully revealed (or whitespace): paint the plain galley.
                    _ => {
                        components::paint_galley(
                            c.painter(),
                            pos2(seg.x, layout.title_pos.y),
                            &seg.galley,
                        );
                    }
                }
            }
            components::stroke_border_bottom(c.painter(), layout.header, pal.border_weak, ppp);
            if let Some((rect, label)) = &layout.badge {
                components::tag(
                    &mut c,
                    Id::unique("badge"),
                    rect.min,
                    TagScheme::Warning,
                    true,
                    label,
                    None,
                );
            }

            // ---- Main (scrolled card or empty state) ----------------------
            c.ui.set_clip_rect(layout.main);
            if let Some(cl) = &layout.card_layout {
                let req = self
                    .queue
                    .front()
                    .expect("card layout implies a current request");
                self.paint_card(&mut c, cl, req, &mut fb_text, scroll_offset);
            } else if let Some(el) = &layout.empty {
                self.paint_empty(&mut c, el, scroll_offset);
            }
            c.ui.set_clip_rect(layout.screen);

            // ---- Footer ---------------------------------------------------
            components::stroke_border_top(c.painter(), layout.footer, pal.border_weak, ppp);
            components::paint_galley(
                c.painter(),
                layout.footer_text.pos,
                &layout.footer_text.galley,
            );
            if let Some(row) = &layout.cooldown_row {
                components::status_dot(
                    &mut c,
                    row.dot_x,
                    Rect::from_min_size(pos2(row.dot_x, row.row_top), vec2(0.0, row.row_h)),
                );
                components::paint_galley(c.painter(), row.text.pos, &row.text.galley);
            }

            // ---- Scrollbar (overlay, inside main's right edge) ------------
            self.paint_scrollbar(&mut c, &layout);

            c.press_consumed
        };

        // ---- Events -------------------------------------------------------
        for ev in events.drain(..) {
            match ev {
                AppEvent::Approve => self.decide(&ctx, Decision::Approve),
                AppEvent::Deny => self.decide(&ctx, Decision::Deny),
                AppEvent::SubmitFeedback(text) => self.decide(&ctx, Decision::Feedback(text)),
                AppEvent::ToggleFeedback => {
                    self.feedback_open = !self.feedback_open;
                    ctx.request_repaint();
                }
            }
        }

        // Click-blur: a primary press no control consumed blurs the focused
        // control (browser behavior).
        if ctx.input(|i| i.pointer.primary_pressed()) && !press_consumed {
            focus.blur();
            for id in [
                self.ta_id,
                button_id(FocusId::SubmitFeedback),
                button_id(FocusId::GiveFeedback),
                button_id(FocusId::Deny),
                button_id(FocusId::Approve),
            ] {
                ctx.memory_mut(|m| m.surrender_focus(id));
            }
        }

        // Keep repainting while a button hover tween or the scrollbar fade
        // is still in flight.
        let mut any_tween = false;
        for fid in [
            FocusId::SubmitFeedback,
            FocusId::GiveFeedback,
            FocusId::Deny,
            FocusId::Approve,
        ] {
            let id = button_id(fid);
            if let Some(t) = ctx.data_mut(|d| d.get_temp::<Tween>(id)) {
                if !t.at_target(now) {
                    any_tween = true;
                }
            }
        }
        if any_tween || self.scroll.fading(now) {
            hover_until = now + 0.25;
        }

        // ---- Restore ------------------------------------------------------
        self.events = events;
        self.focus = focus;
        self.feedback_text = fb_text;
        self.hover_repaint_until = hover_until;
    }

    /// Lay out the title segments for painting, building the per-segment
    /// blur textures lazily (they persist across frames and are rebuilt when
    /// the pixel scale or the responsive title spec changes).
    fn prepare_title(&mut self, c: &Ctx, layout: &Layout) -> Vec<TitleSegPaint> {
        let ctx = c.ctx().clone();
        let ppp = ctx.pixels_per_point();
        if self.title_ppp != ppp || self.title_shared.len() != self.title_segs.len() {
            self.title_shared.clear();
            self.title_ppp = ppp;
        }
        while self.title_shared.len() < self.title_segs.len() {
            self.title_shared.push(None);
        }
        let mut out = Vec::with_capacity(self.title_segs.len());
        let mut x = layout.title_pos.x;
        for (i, seg) in self.title_segs.iter().enumerate() {
            let state = if self.reduce_motion {
                None
            } else {
                match self.title_fx.segment_state(i, self.frame_now) {
                    Some(st) if st.opacity >= 1.0 && st.blur <= 0.0 && st.y_offset <= 0.0 => None,
                    Some(st) => Some(st),
                    // Not started yet: the pre-reveal state (invisible).
                    None => Some(SegmentState {
                        opacity: 0.0,
                        y_offset: 20.0,
                        blur: 12.0,
                    }),
                }
            };
            let tex = if seg.text.trim().is_empty() {
                None
            } else {
                if self.title_shared[i].is_none() {
                    let atlas = ctx.fonts(|f| f.image());
                    if let Some((image, ink_min, pad)) =
                        shaders::build_segment_texture(&seg.galley, &atlas, ppp)
                    {
                        let size = [image.size[0] as f32, image.size[1] as f32];
                        self.title_shared[i] = Some(TitleTex {
                            shared: Arc::new(shaders::TitleBlurShared::new(
                                Arc::clone(&self.effects),
                                image,
                            )),
                            ink_min,
                            pad,
                            size,
                        });
                    }
                }
                self.title_shared[i].clone()
            };
            out.push(TitleSegPaint {
                x,
                galley: seg.galley.clone(),
                state,
                tex,
            });
            x += seg.advance;
        }
        out
    }

    /// Paint the request card (all positions in `cl` are unscrolled).
    #[allow(clippy::too_many_arguments)]
    fn paint_card(
        &self,
        c: &mut Ctx,
        cl: &CardLayout,
        req: &MessageRequest,
        fb_text: &mut String,
        scroll_offset: f32,
    ) {
        let m = c.m;
        let pal = c.pal;
        let card = sy(scroll_offset, cl.card);

        // Card frame (surface fill + 1px border).
        components::panel_frame(
            c,
            card,
            m.radius_l,
            pal.surface,
            pal.border_weak,
            m.scale(CARD_PAD),
        );

        // (The reference's card `Spotlight` paints nothing — its Tailwind
        // radial-gradient background is invalid at computed-value time, so
        // the div's `background-image` computes to `none`; see
        // `shaders.rs`. There is therefore no overlay to paint here.)

        // Heading row.
        components::paint_galley(c.painter(), py(scroll_offset, cl.heading.pos), &cl.heading.galley);
        components::paint_galley(c.painter(), py(scroll_offset, cl.subline.pos), &cl.subline.galley);
        let (agent_rect, agent_label) = &cl.agent_tag;
        components::tag(
            c,
            Id::unique("agent-tag"),
            py(scroll_offset, agent_rect.min),
            TagScheme::Brand,
            true,
            agent_label,
            Some(Icon::Person),
        );

        // Proposed message.
        components::paint_galley(c.painter(), py(scroll_offset, cl.label_msg.pos), &cl.label_msg.galley);
        let msg_panel = sy(scroll_offset, cl.msg_panel);
        components::panel_frame(
            c,
            msg_panel,
            m.radius_m,
            pal.page,
            pal.border_weak,
            m.scale(MSG_PANEL_PAD),
        );
        components::code_text(
            c,
            cl.msg_text.id,
            py(scroll_offset, cl.msg_text.pos),
            &req.incoming.message,
            pal.text_strong,
            cl.msg_text.wrap,
        );

        // Destination.
        components::paint_galley(c.painter(), py(scroll_offset, cl.label_dest.pos), &cl.label_dest.galley);
        components::icon_at(c, Icon::Hash, py(scroll_offset, cl.dest_hash), 14.0, pal.info_on_bg_medium);
        let (info_rect, info_label) = &cl.info_tag;
        components::tag(
            c,
            Id::unique("dest-info"),
            py(scroll_offset, info_rect.min),
            TagScheme::Info,
            true,
            info_label,
            None,
        );
        if let Some((rect, label)) = &cl.raw_tag {
            components::tag(
                c,
                Id::unique("dest-raw"),
                py(scroll_offset, rect.min),
                TagScheme::Neutral,
                false,
                label,
                None,
            );
        }
        if let Some((rect, label)) = &cl.thread_tag {
            components::tag(
                c,
                Id::unique("dest-thread"),
                py(scroll_offset, rect.min),
                TagScheme::Neutral,
                false,
                label,
                None,
            );
        }
        components::paint_galley(c.painter(), py(scroll_offset, cl.hint_dest.pos), &cl.hint_dest.galley);

        // Reason.
        components::paint_galley(c.painter(), py(scroll_offset, cl.label_reason.pos), &cl.label_reason.galley);
        components::body_text(
            c,
            cl.reason.id,
            py(scroll_offset, cl.reason.pos),
            &req.incoming.reason,
            pal.text_strong,
            cl.reason.wrap,
        );

        // Originating agent conversation.
        components::paint_galley(c.painter(), py(scroll_offset, cl.label_conv.pos), &cl.label_conv.galley);
        let conv_panel = sy(scroll_offset, cl.conv_panel);
        components::panel_frame(
            c,
            conv_panel,
            m.radius_m,
            pal.page,
            pal.border_weak,
            m.scale(CONV_PANEL_PAD),
        );
        components::icon_at(c, Icon::MessagesSquare, py(scroll_offset, cl.conv_icon), 16.0, pal.text_strong);
        components::paint_galley(c.painter(), py(scroll_offset, cl.conv_title.pos), &cl.conv_title.galley);
        components::code_xs_text(
            c,
            cl.conv_link.id,
            py(scroll_offset, cl.conv_link.pos),
            &req.incoming.conversation.id_or_link,
            pal.text_medium,
            cl.conv_link.wrap,
        );
        components::body_text(
            c,
            cl.conv_summary.id,
            py(scroll_offset, cl.conv_summary.pos),
            &req.incoming.conversation.summary,
            pal.text_medium,
            cl.conv_summary.wrap,
        );
        components::paint_galley(c.painter(), py(scroll_offset, cl.hint_conv.pos), &cl.hint_conv.galley);

        // Feedback + actions column.
        if let Some(ta_box) = cl.ta_box {
            let ta_pos = py(scroll_offset, ta_box.min);
            components::textarea(
                c,
                self.ta_id,
                ta_pos,
                ta_box.width(),
                fb_text,
                TEXTAREA_LABEL,
                TEXTAREA_PLACEHOLDER,
                self.deciding,
            );
            let sb_rect = cl
                .submit_btn
                .as_ref()
                .expect("submit button is laid out with the textarea");
            let activated = components::button(
                c,
                button_id(FocusId::SubmitFeedback),
                py(scroll_offset, sb_rect.min),
                ButtonVariant::Secondary,
                BTN_SUBMIT_FEEDBACK,
                Some(Icon::Chat),
                self.deciding || fb_text.trim().is_empty(),
                FocusId::SubmitFeedback,
            );
            if activated {
                c.events.push(AppEvent::SubmitFeedback(fb_text.trim().to_string()));
            }
            if let Some(e) = &cl.error_text {
                components::paint_galley(c.painter(), py(scroll_offset, e.pos), &e.galley);
            }
        } else if let Some(e) = &cl.error_text {
            components::paint_galley(c.painter(), py(scroll_offset, e.pos), &e.galley);
        }

        // Action buttons (right-aligned, wrapping).
        let give = components::button(
            c,
            button_id(FocusId::GiveFeedback),
            py(scroll_offset, cl.btn_give.min),
            ButtonVariant::Secondary,
            BTN_GIVE_FEEDBACK,
            Some(Icon::Chat),
            self.deciding,
            FocusId::GiveFeedback,
        );
        if give {
            c.events.push(AppEvent::ToggleFeedback);
        }
        let deny = components::button(
            c,
            button_id(FocusId::Deny),
            py(scroll_offset, cl.btn_deny.min),
            ButtonVariant::Danger,
            BTN_DENY,
            Some(Icon::Close),
            self.deciding,
            FocusId::Deny,
        );
        if deny {
            c.events.push(AppEvent::Deny);
        }
        let approve = components::button(
            c,
            button_id(FocusId::Approve),
            py(scroll_offset, cl.btn_approve.min),
            ButtonVariant::Primary,
            BTN_APPROVE,
            Some(Icon::Check),
            self.deciding,
            FocusId::Approve,
        );
        if approve {
            c.events.push(AppEvent::Approve);
        }
    }

    /// Paint the empty state (positions are unscrolled).
    fn paint_empty(&self, c: &mut Ctx, el: &EmptyLayout, scroll_offset: f32) {
        let m = c.m;
        components::panel_frame(
            c,
            sy(scroll_offset, el.card),
            m.radius_l,
            c.pal.surface,
            c.pal.border_weak,
            m.scale(EMPTY_PAD),
        );
        components::icon_at(c, Icon::Inbox, py(scroll_offset, el.icon), el.icon_size, c.pal.text_medium);
        // Centered galleys are painted at their center x.
        components::paint_galley(c.painter(), py(scroll_offset, el.title.pos), &el.title.galley);
        components::paint_galley(c.painter(), py(scroll_offset, el.sub.pos), &el.sub.galley);
    }

    /// Paint the overlay scrollbar for the main region (macOS-style: an
    /// overlay thumb, no layout space, fading out after 0.5s idle).
    fn paint_scrollbar(&mut self, c: &mut Ctx, layout: &Layout) {
        let m = c.m;
        let now = c.now;
        if self.max_scroll <= 0.0 {
            return;
        }
        let track_w = m.scrollbar_w;
        let track = Rect::from_min_max(
            pos2(layout.main.right() - track_w, layout.main.top()),
            layout.main.max,
        );
        let main_h = layout.main.height();
        let scroll_h = self.content_height + 2.0 * m.scale(MAIN_PAD);
        let thumb_h = ((main_h * main_h) / scroll_h)
            .max(m.scale(SCROLLBAR_MIN_THUMB))
            .min(main_h);
        let travel = main_h - thumb_h;
        let thumb_top = layout.main.top()
            + if travel > 0.0 {
                (self.scroll_offset / self.max_scroll) * travel
            } else {
                0.0
            };
        let thumb = Rect::from_min_size(pos2(track.left(), thumb_top), vec2(track_w, thumb_h));

        // Drag the thumb.
        let grab_id = Id::unique("sag-scrollbar-grab");
        let resp = c
            .ui
            .interact(thumb, Id::unique("sag-scrollbar-thumb"), Sense::click_and_drag());
        if resp.drag_started() {
            if let Some(pos) = c.ctx().input(|i| i.pointer.interact_pos()) {
                c.ctx()
                    .data_mut(|d| d.insert_temp(grab_id, pos.y - thumb_top));
            }
            c.press_consumed = true;
        }
        if resp.is_pointer_button_down_on() {
            c.press_consumed = true;
        }
        if resp.dragged() {
            c.press_consumed = true;
            let grab: f32 = c.ctx().data_mut(|d| d.get_temp(grab_id)).unwrap_or(0.0);
            if let Some(pos) = c.ctx().input(|i| i.pointer.interact_pos()) {
                let new_top = (pos.y - grab).clamp(layout.main.top(), layout.main.top() + travel);
                self.scroll_offset = if travel > 0.0 {
                    ((new_top - layout.main.top()) / travel) * self.max_scroll
                } else {
                    0.0
                };
            }
            self.scroll.activity(now);
            c.ctx().request_repaint();
        }
        let hovered = resp.hovered() || resp.dragged();
        if hovered {
            self.scroll.activity(now);
        }
        self.scroll.hover.set(if hovered { 1.0 } else { 0.0 }, now);

        let fade = if c.reduce_motion {
            self.scroll.fade.target_value()
        } else {
            self.scroll.fade.value(now)
        };
        if fade > 0.0 {
            let hover_t = if c.reduce_motion {
                self.scroll.hover.target_value()
            } else {
                self.scroll.hover.value(now)
            };
            let alpha = (SCROLLBAR_THUMB_ALPHA
                + (SCROLLBAR_THUMB_ALPHA_HOVER - SCROLLBAR_THUMB_ALPHA) * hover_t)
                * fade;
            if alpha > 0.0 {
                components::fill_rounded(
                    c.painter(),
                    thumb,
                    track_w / 2.0,
                    Color32::from_rgba_unmultiplied(149, 149, 149, (alpha * 255.0).round() as u8),
                );
            }
        }
    }
}

/// Compute the request-card layout. `card_x`/`card_top`/`card_w` are the
/// card's border-box position; the returned layout's `card` rect is fully
/// sized (height = content + 2 × (1px border + padding)).
#[allow(clippy::too_many_lines)]
fn compute_card(
    app: &ApprovalApp,
    ctx: &Context,
    m: &Metrics,
    req: &MessageRequest,
    card_x: f32,
    card_top: f32,
    card_w: f32,
) -> CardLayout {
    let pal = &app.pal;
    let pad = m.scale(CARD_PAD);
    let card_gap = m.scale(CARD_GAP);
    let section_gap = m.scale(SECTION_GAP);
    let content_x = card_x + 1.0 + pad;
    let cw = card_w - 2.0 * (1.0 + pad);
    let mut y = card_top + 1.0 + pad;
    let incoming = &req.incoming;

    // ---- Heading row (between/start, wrap) --------------------------------
    let heading = components::galley(ctx, m.heading, HEADING, pal.text_strong, cw, false);
    let subline_text = format!(
        "Request {} \u{b7} received {}",
        req.id,
        sys::format_time_medium(req.received_at)
    );
    let subline = components::galley(ctx, m.body_xs, &subline_text, pal.text_medium, cw, false);
    let left_w = heading.rect.width().max(subline.rect.width());
    let heading_h = components::block_height(&heading, m.heading);
    let subline_h = components::block_height(&subline, m.body_xs);
    let left_h = heading_h + m.sp4 + subline_h;
    let agent_label = incoming.agent.clone();
    let agent_w = components::tag_width(
        ctx,
        m,
        &agent_label,
        Some(Icon::Person),
        pal.brand_on_bg_medium,
    );
    let agent_h = tag_h_m(m);
    let (row_rects, row_h) = components::layout_row(
        &[(left_w, left_h), (agent_w, agent_h)],
        pos2(content_x, y),
        cw,
        m.sp16,
        Justify::Between,
        RowAlign::Start,
    );
    let left = row_rects[0];
    let heading_pos = left.min;
    let subline_pos = pos2(left.min.x, left.min.y + heading_h + m.sp4);
    let agent_rect = row_rects[1];
    y += row_h + card_gap;

    // ---- Proposed message section ------------------------------------------
    let label_msg = components::galley(ctx, m.label_xs, LABEL_MESSAGE, pal.text_medium, cw, false);
    let label_msg_pos = pos2(content_x, y);
    y += components::block_height(&label_msg, m.label_xs) + section_gap;
    let msg_pad = m.scale(MSG_PANEL_PAD);
    let msg_wrap = cw - 2.0 * (1.0 + msg_pad);
    let msg_galley = components::galley(
        ctx,
        m.code_s,
        &incoming.message,
        pal.text_strong,
        msg_wrap,
        true,
    );
    let msg_text_h = components::block_height(&msg_galley, m.code_s);
    let msg_panel_h = msg_text_h + 2.0 * (1.0 + msg_pad);
    let msg_panel = Rect::from_min_size(pos2(content_x, y), vec2(cw, msg_panel_h));
    let msg_text = SelText {
        id: Id::unique("msg-text").with(&req.id),
        pos: pos2(content_x + 1.0 + msg_pad, y + 1.0 + msg_pad),
        wrap: msg_wrap,
        h: msg_text_h,
    };
    y += msg_panel_h + card_gap;

    // ---- Destination section ------------------------------------------------
    let label_dest = components::galley(ctx, m.label_xs, LABEL_DESTINATION, pal.text_medium, cw, false);
    let label_dest_pos = pos2(content_x, y);
    y += components::block_height(&label_dest, m.label_xs) + section_gap;
    // Row (start/center, wrap, gap space-8): [hash + info tag] [raw] [thread].
    let hash_side = 14.0; // literal lucide `size={14}` — absolute
    let info_label = req
        .destination_display
        .clone()
        .unwrap_or_else(|| incoming.destination.clone());
    let info_w = components::tag_width(ctx, m, &info_label, None, pal.info_on_bg_medium);
    let info_h = tag_h_m(m);
    let inner_w = hash_side + m.sp4 + info_w;
    let mut items: Vec<(f32, f32)> = vec![(inner_w, info_h)];
    let mut labels: Vec<String> = vec![info_label.clone()];
    let has_raw = req.destination_display.is_some();
    if has_raw {
        let label = incoming.destination.clone();
        let w = components::tag_width(ctx, m, &label, None, pal.neutral_tag_text);
        items.push((w, tag_h_s(m)));
        labels.push(label);
    }
    let has_thread = incoming.thread_ts.is_some();
    if let Some(ts) = &incoming.thread_ts {
        let label = format!("thread {ts}");
        let w = components::tag_width(ctx, m, &label, None, pal.neutral_tag_text);
        items.push((w, tag_h_s(m)));
        labels.push(label);
    }
    let (dest_rects, dest_row_h) = components::layout_row(
        &items,
        pos2(content_x, y),
        cw,
        m.sp8,
        Justify::Start,
        RowAlign::Center,
    );
    let inner_rect = dest_rects[0];
    let dest_hash = pos2(
        inner_rect.min.x,
        inner_rect.min.y + (inner_rect.height() - hash_side) / 2.0,
    );
    let info_rect = Rect::from_min_size(
        pos2(inner_rect.min.x + hash_side + m.sp4, inner_rect.min.y),
        vec2(info_w, info_h),
    );
    let mut idx = 1;
    let raw_tag = if has_raw {
        let rect = dest_rects[idx];
        idx += 1;
        Some((rect, labels[1].clone()))
    } else {
        None
    };
    let thread_tag = if has_thread {
        let rect = dest_rects[idx];
        Some((rect, labels[idx].clone()))
    } else {
        None
    };
    y += dest_row_h + section_gap;
    let hint_dest = components::galley(ctx, m.body_xs, HINT_DESTINATION, pal.text_medium, cw, false);
    let hint_dest_pos = pos2(content_x, y);
    y += components::block_height(&hint_dest, m.body_xs) + card_gap;

    // ---- Reason section ------------------------------------------------------
    let label_reason = components::galley(ctx, m.label_xs, LABEL_REASON, pal.text_medium, cw, false);
    let label_reason_pos = pos2(content_x, y);
    y += components::block_height(&label_reason, m.label_xs) + section_gap;
    let reason_galley = components::galley(
        ctx,
        m.body_s,
        &incoming.reason,
        pal.text_strong,
        cw,
        false,
    );
    let reason_h = components::block_height(&reason_galley, m.body_s);
    let reason = SelText {
        id: Id::unique("reason").with(&req.id),
        pos: pos2(content_x, y),
        wrap: cw,
        h: reason_h,
    };
    y += reason_h + card_gap;

    // ---- Conversation section -------------------------------------------------
    let label_conv = components::galley(
        ctx,
        m.label_xs,
        LABEL_CONVERSATION,
        pal.text_medium,
        cw,
        false,
    );
    let label_conv_pos = pos2(content_x, y);
    y += components::block_height(&label_conv, m.label_xs) + section_gap;
    let conv_pad = m.scale(CONV_PANEL_PAD);
    let inner_x = content_x + 1.0 + conv_pad;
    let inner_w = cw - 2.0 * (1.0 + conv_pad);
    let mut cy = y + 1.0 + conv_pad;
    // Header row: icon + title (center).
    let conv_icon_side = 16.0; // literal lucide `size={16}` — absolute
    let conv_title_galley = components::galley(
        ctx,
        m.body_strong_s,
        &incoming.conversation.title,
        pal.text_strong,
        inner_w - conv_icon_side - m.sp8,
        false,
    );
    let conv_title_h = components::block_height(&conv_title_galley, m.body_strong_s);
    let conv_row_h = conv_title_h.max(conv_icon_side);
    let conv_icon = pos2(inner_x, cy + (conv_row_h - conv_icon_side) / 2.0);
    let conv_title_pos = pos2(
        inner_x + conv_icon_side + m.sp8,
        cy + (conv_row_h - conv_title_h) / 2.0,
    );
    cy += conv_row_h + section_gap;
    // Link (code-xs, overflow-wrap anywhere).
    let link_galley = components::galley(
        ctx,
        m.code_xs,
        &incoming.conversation.id_or_link,
        pal.text_medium,
        inner_w,
        true,
    );
    let conv_link_h = components::block_height(&link_galley, m.code_xs);
    let conv_link = SelText {
        id: Id::unique("conv-link").with(&req.id),
        pos: pos2(inner_x, cy),
        wrap: inner_w,
        h: conv_link_h,
    };
    cy += conv_link_h + section_gap;
    // Summary (body-s).
    let summary_galley = components::galley(
        ctx,
        m.body_s,
        &incoming.conversation.summary,
        pal.text_medium,
        inner_w,
        false,
    );
    let conv_summary_h = components::block_height(&summary_galley, m.body_s);
    let conv_summary = SelText {
        id: Id::unique("conv-summary").with(&req.id),
        pos: pos2(inner_x, cy),
        wrap: inner_w,
        h: conv_summary_h,
    };
    cy += conv_summary_h;
    // Border-box height: 1px border + padding + content + padding + 1px border.
    let conv_panel_h = (cy - (y + 1.0 + conv_pad)) + 2.0 * (1.0 + conv_pad);
    let conv_panel = Rect::from_min_size(pos2(content_x, y), vec2(cw, conv_panel_h));
    y += conv_panel_h + section_gap;
    let hint_conv = components::galley(ctx, m.body_xs, HINT_CONVERSATION, pal.text_medium, cw, false);
    let hint_conv_pos = pos2(content_x, y);
    y += components::block_height(&hint_conv, m.body_xs) + card_gap;

    // ---- Actions column (gap space-12) ------------------------------------------
    let actions_gap = m.scale(ACTIONS_GAP);
    let mut ta_box = None;
    let mut submit_btn = None;
    let mut error_text = None;
    if app.feedback_open {
        // The textarea auto-grows with its text (measure like the component).
        let text_w = cw - 2.0 * (1.0 + components::field::well(m) + components::field::text_pad_x(m));
        let measure = components::galley(ctx, m.field, &app.feedback_text, pal.text_strong, text_w, false);
        let lines = measure.rows.len().max(1);
        let box_h = components::field::box_height(m, lines);
        ta_box = Some(Rect::from_min_size(pos2(content_x, y), vec2(cw, box_h)));
        y += box_h + section_gap; // feedback column gap = space-8
        let btn_h = m.sp32;
        let btn_w = components::button_width(
            ctx,
            m,
            ButtonVariant::Secondary,
            BTN_SUBMIT_FEEDBACK,
            Some(Icon::Chat),
            pal.text_strong,
        );
        submit_btn = Some(Rect::from_min_size(
            pos2(content_x + cw - btn_w, y),
            vec2(btn_w, btn_h),
        ));
        y += btn_h;
        if app.submit_error {
            y += actions_gap;
            let e = components::galley(ctx, m.body_xs, SUBMIT_ERROR, pal.danger_on_bg_medium, cw, false);
            error_text = Some(TextBlock {
                galley: e,
                pos: pos2(content_x, y),
            });
            y += components::block_height(&error_text.as_ref().unwrap().galley, m.body_xs);
        }
        y += actions_gap;
    } else if app.submit_error {
        let e = components::galley(ctx, m.body_xs, SUBMIT_ERROR, pal.danger_on_bg_medium, cw, false);
        error_text = Some(TextBlock {
            galley: e,
            pos: pos2(content_x, y),
        });
        y += components::block_height(&error_text.as_ref().unwrap().galley, m.body_xs);
        y += actions_gap;
    }

    // Action buttons row (end, wrap, gap space-8).
    let btn_h = m.sp32;
    let give_w = components::button_width(
        ctx,
        m,
        ButtonVariant::Secondary,
        BTN_GIVE_FEEDBACK,
        Some(Icon::Chat),
        pal.text_strong,
    );
    let deny_w = components::button_width(
        ctx,
        m,
        ButtonVariant::Danger,
        BTN_DENY,
        Some(Icon::Close),
        pal.danger_on_solid_strong,
    );
    let approve_w = components::button_width(
        ctx,
        m,
        ButtonVariant::Primary,
        BTN_APPROVE,
        Some(Icon::Check),
        pal.brand_on_solid_strong,
    );
    let (btn_rects, btn_row_h) = components::layout_row(
        &[(give_w, btn_h), (deny_w, btn_h), (approve_w, btn_h)],
        pos2(content_x, y),
        cw,
        m.sp8,
        Justify::End,
        RowAlign::Center,
    );
    y += btn_row_h;
    let content_h = y - (card_top + 1.0 + pad);

    CardLayout {
        card: Rect::from_min_size(
            pos2(card_x, card_top),
            vec2(card_w, content_h + 2.0 * (1.0 + pad)),
        ),
        content_x,
        cw,
        heading: TextBlock {
            galley: heading,
            pos: heading_pos,
        },
        subline: TextBlock {
            galley: subline,
            pos: subline_pos,
        },
        subline_text,
        agent_tag: (agent_rect, agent_label),
        label_msg: TextBlock {
            galley: label_msg,
            pos: label_msg_pos,
        },
        msg_panel,
        msg_text,
        label_dest: TextBlock {
            galley: label_dest,
            pos: label_dest_pos,
        },
        dest_hash,
        info_tag: (info_rect, info_label),
        raw_tag,
        thread_tag,
        hint_dest: TextBlock {
            galley: hint_dest,
            pos: hint_dest_pos,
        },
        label_reason: TextBlock {
            galley: label_reason,
            pos: label_reason_pos,
        },
        reason,
        label_conv: TextBlock {
            galley: label_conv,
            pos: label_conv_pos,
        },
        conv_panel,
        conv_icon,
        conv_title: TextBlock {
            galley: conv_title_galley,
            pos: conv_title_pos,
        },
        conv_link,
        conv_summary,
        hint_conv: TextBlock {
            galley: hint_conv,
            pos: hint_conv_pos,
        },
        ta_box,
        submit_btn,
        error_text,
        btn_give: btn_rects[0],
        btn_deny: btn_rects[1],
        btn_approve: btn_rects[2],
        content_h,
    }
}

/// Compute the empty-state layout.
fn compute_empty(
    app: &ApprovalApp,
    ctx: &Context,
    m: &Metrics,
    card_x: f32,
    card_top: f32,
    card_w: f32,
) -> EmptyLayout {
    let pal = &app.pal;
    let pad = m.scale(EMPTY_PAD);
    let center_x = card_x + card_w / 2.0;
    let cw = card_w - 2.0 * (1.0 + pad);
    let mut y = card_top + 1.0 + pad;

    let icon_size = m.icon_l;
    let icon = pos2(center_x - icon_size / 2.0, y);
    y += icon_size + m.scale(EMPTY_GAP);

    // Centered like CSS `text-align: center`: the line box is centered
    // exactly. (egui's `Align::Center` rounds the row position to whole
    // pixels, so a left-aligned galley painted at `center − width/2` — with
    // the galley's exact unrounded width — is the faithful equivalent.)
    let title = components::galley(ctx, m.body_strong_m, EMPTY_TITLE, pal.text_strong, cw, false);
    let title_pos = pos2(center_x - title.rect.width() / 2.0, y);
    y += components::block_height(&title, m.body_strong_m) + m.scale(EMPTY_GAP);

    let sub = components::galley(ctx, m.body_s, EMPTY_SUB, pal.text_medium, cw, false);
    let sub_pos = pos2(center_x - sub.rect.width() / 2.0, y);
    y += components::block_height(&sub, m.body_s);

    let content_h = y - (card_top + 1.0 + pad);
    EmptyLayout {
        card: Rect::from_min_size(
            pos2(card_x, card_top),
            vec2(card_w, content_h + 2.0 * (1.0 + pad)),
        ),
        icon,
        icon_size,
        title: TextBlock {
            galley: title,
            pos: title_pos,
        },
        sub: TextBlock {
            galley: sub,
            pos: sub_pos,
        },
        content_h,
    }
}
