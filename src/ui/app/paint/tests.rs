//! Layout regression tests: the capture-verified reference geometry
//! (720×780 @ root 15, 1000×780 @ root 16, 520×560 narrow) encoded as
//! font-independent invariants of `compute_layout`.

use super::*;
use crate::ui::demo;
use crate::ui::text;
use crate::ui::theme::Theme;
use crate::ui::UiState;
use std::sync::Mutex;

fn app_with(requests: Vec<MessageRequest>) -> ApprovalApp {
    let state = Arc::new(UiState::new());
    let mut app = ApprovalApp::new(
        state,
        Arc::new(Mutex::new(None)),
        Theme::Light,
        false,
        true,
        super::super::DemoFlags::default(),
    );
    for r in requests {
        app.queue.push_back(r);
    }
    app
}

/// A context with fonts installed and a fixed viewport size.
fn ctx_with_viewport(w: f32, h: f32) -> Context {
    let ctx = egui::Context::default();
    text::install_fonts(&ctx);
    let mut out = ctx.run_pass(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(w, h))),
            ..Default::default()
        },
        |_| {},
    );
    // The font-atlas delta is applied by the real renderer; drop it here.
    out.textures_delta.clear();
    ctx
}

fn layout_for(app: &ApprovalApp, w: f32, h: f32) -> Layout {
    let ctx = ctx_with_viewport(w, h);
    let m = Metrics::new(w);
    app.compute_layout(&ctx, &m, 0.0)
}

fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

#[test]
fn metrics_scale_with_the_responsive_root() {
    let m = Metrics::new(720.0);
    assert!(approx(m.s, 1.0));
    assert!(approx(m.title.size, 33.75));
    assert!(approx(m.title.line_height, 37.5));
    let wide = Metrics::new(1000.0);
    assert!(approx(wide.s, 16.0 / 15.0));
    assert!(approx(wide.title.size, 44.0));
    assert!(approx(wide.title.line_height, 48.0));
    let huge = Metrics::new(1200.0);
    assert!(approx(huge.title.size, 48.0));
    assert!(approx(huge.title.line_height, 52.0));
    // Card max width: 64rem at ≤1024px, 75rem above.
    assert!(approx(Metrics::new(720.0).card_max_w, 960.0));
    assert!(approx(Metrics::new(1000.0).card_max_w, 1024.0));
    assert!(approx(Metrics::new(1200.0).card_max_w, 1200.0));
}

#[test]
fn header_card_and_footer_geometry_at_720() {
    let mut app = app_with(vec![demo::demo_request()]);
    app.badge.jump(1, 0.0);
    let l = layout_for(&app, 720.0, 780.0);

    // Header: 15 + 37.5 + 15 + 1px border.
    assert!(approx(l.header.height(), 68.5));
    assert!(approx(l.main.top(), 68.5));
    // Slack icon (20px literal), vertically centered in the row.
    assert!(approx(l.slack_icon.x, 22.5));
    assert!(approx(l.slack_icon.y, 23.75));
    // Title line box.
    assert!(approx(l.title_pos.x, 50.0));
    assert!(approx(l.title_pos.y, 15.0));
    // Badge: right edge at 720 − 22.5, vertically centered.
    let (badge_rect, badge_label) = l.badge.as_ref().expect("badge with a queued request");
    assert!(approx(badge_rect.right(), 697.5));
    assert!(approx(badge_rect.top(), 23.375));
    assert!(approx(badge_rect.height(), 20.75));
    assert_eq!(badge_label, "1 waiting");

    // Card.
    let cl = l.card_layout.as_ref().expect("card layout");
    assert!(approx(cl.card.min.x, 22.5));
    assert!(approx(cl.card.top(), 91.0));
    assert!(approx(cl.card.width(), 675.0));
    assert!(approx(cl.content_x, 46.0));
    assert!(approx(cl.cw, 628.0));

    // Heading row: heading line box at the content top-left; agent tag at
    // the row's right edge, top-aligned.
    assert!(approx(cl.heading.pos.x, 46.0));
    assert!(approx(cl.heading.pos.y, 114.5));
    let m = Metrics::new(720.0);
    assert!(approx(components::block_height(&cl.heading.galley, m.heading), 37.5));
    assert!(approx(cl.subline.pos.y, 114.5 + 37.5 + 3.75));
    assert!(cl.subline_text.starts_with("Request demo-0001 · received "));
    let (agent_rect, agent_label) = &cl.agent_tag;
    assert!(approx(agent_rect.right(), 674.0));
    assert!(approx(agent_rect.top(), 114.5));
    assert!(approx(agent_rect.height(), 20.75));
    assert_eq!(agent_label, "claude-code");

    // Message section: label, then the panel (page bg, 1px border,
    // space-12 padding, radius-m).
    let heading_row_h = 37.5 + 3.75 + 15.0;
    assert!(approx(cl.label_msg.pos.y, 114.5 + heading_row_h + 18.75));
    assert!(approx(components::block_height(&cl.label_msg.galley, m.label_xs), 13.125));
    let msg_panel_top = cl.label_msg.pos.y + 13.125 + 7.5;
    assert!(approx(cl.msg_panel.top(), msg_panel_top));
    assert!(approx(cl.msg_panel.width(), 628.0));
    assert!(approx(cl.msg_text.pos.x, 46.0 + 1.0 + 11.25));
    assert!(approx(cl.msg_text.pos.y, msg_panel_top + 1.0 + 11.25));
    assert!(approx(cl.msg_text.wrap, 628.0 - 2.0 * 12.25));
    // The panel wraps its text: height = text + 2 × (border + padding).
    assert!(approx(cl.msg_panel.height(), cl.msg_text.h + 2.0 * 12.25));
    assert!(approx(cl.msg_panel.height(), 54.5));

    // Destination row: hash icon, info tag, raw tag, thread tag.
    let dest_label_y = cl.msg_panel.bottom() + 18.75;
    assert!(approx(cl.label_dest.pos.y, dest_label_y));
    let dest_row_top = dest_label_y + 13.125 + 7.5;
    assert!(approx(cl.dest_hash.x, 46.0));
    assert!(approx(cl.dest_hash.y, dest_row_top + (20.75 - 14.0) / 2.0));
    let (info_rect, info_label) = &cl.info_tag;
    assert!(approx(info_rect.min.x, 46.0 + 14.0 + 3.75));
    assert!(approx(info_rect.min.y, dest_row_top));
    assert!(approx(info_rect.height(), 20.75));
    assert_eq!(info_label, "Drake Thomsen (@drake.thomsen)");
    let (raw_rect, raw_label) = cl
        .raw_tag
        .as_ref()
        .expect("resolved destination shows the raw id");
    assert!(approx(raw_rect.min.x, info_rect.right() + 7.5));
    assert!(approx(raw_rect.min.y, dest_row_top + (20.75 - 19.0) / 2.0));
    assert!(approx(raw_rect.height(), 19.0));
    assert_eq!(raw_label, "U09CV6A19EH");
    let (thread_rect, thread_label) = cl.thread_tag.as_ref().expect("thread badge");
    assert!(approx(thread_rect.min.x, raw_rect.right() + 7.5));
    assert_eq!(thread_label, "thread 1729612800.123456");
    // Destination hint below the row.
    let hint_y = dest_row_top + 20.75 + 7.5;
    assert!(approx(cl.hint_dest.pos.y, hint_y));

    // Reason section.
    let reason_label_y =
        cl.hint_dest.pos.y + components::block_height(&cl.hint_dest.galley, m.body_xs) + 18.75;
    assert!(approx(cl.label_reason.pos.y, reason_label_y));
    assert!(approx(cl.reason.pos.y, reason_label_y + 13.125 + 7.5));
    assert!(approx(cl.reason.pos.x, 46.0));
    assert!(approx(cl.reason.wrap, 628.0));

    // Conversation section.
    let conv_label_y = cl.reason.pos.y + cl.reason.h + 18.75;
    assert!(approx(cl.label_conv.pos.y, conv_label_y));
    let conv_panel_top = conv_label_y + 13.125 + 7.5;
    assert!(approx(cl.conv_panel.top(), conv_panel_top));
    assert!(approx(cl.conv_panel.top(), 442.875));
    assert!(approx(cl.conv_panel.width(), 628.0));
    // Reference (capture-derived): the panel is 110.75 tall — content
    // (16.875 + 7.5 + 13.125 + 7.5 + 33.75) plus 2 × (1px border + 15 pad).
    assert!(approx(cl.conv_panel.height(), 110.75));
    let conv_inner_x = 46.0 + 1.0 + 15.0;
    assert!(approx(cl.conv_icon.x, conv_inner_x));
    assert!(approx(cl.conv_title.pos.x, conv_inner_x + 16.0 + 7.5));
    assert!(approx(cl.conv_link.pos.x, conv_inner_x));
    assert!(approx(cl.conv_link.wrap, 628.0 - 2.0 * 16.0));
    assert!(approx(cl.conv_summary.pos.x, conv_inner_x));
    // The link sits between the title row and the summary (gaps of space-8).
    assert!(approx(
        cl.conv_link.pos.y,
        cl.conv_panel.top() + 1.0 + 15.0
            + components::block_height(&cl.conv_title.galley, m.body_strong_s).max(16.0)
            + 7.5
    ));
    assert!(approx(
        cl.conv_summary.pos.y,
        cl.conv_link.pos.y + cl.conv_link.h + 7.5
    ));
    // Panel height wraps its content: pad + rows + gaps + pad + border.
    let conv_content_h = cl.conv_panel.height() - 2.0 * (15.0 + 1.0);
    assert!(approx(conv_content_h, 16.875 + 7.5 + 13.125 + 7.5 + 33.75));
    // The conversation hint sits below the panel (reference: y = 561.125).
    assert!(approx(cl.hint_conv.pos.y, 561.125));

    // No feedback: no textarea, no submit button, no error.
    assert!(cl.ta_box.is_none());
    assert!(cl.submit_btn.is_none());
    assert!(cl.error_text.is_none());

    // Buttons row: right-aligned, gap 7.5, height 30 (reference: y = 594.875
    // for the feedback-closed card).
    let buttons_top =
        cl.hint_conv.pos.y + components::block_height(&cl.hint_conv.galley, m.body_xs) + 18.75;
    assert!(approx(cl.btn_give.top(), buttons_top));
    assert!(approx(cl.btn_give.top(), 594.875));
    assert!(approx(cl.btn_approve.right(), 674.0));
    assert!(approx(cl.btn_deny.right(), cl.btn_approve.left() - 7.5));
    assert!(approx(cl.btn_give.right(), cl.btn_deny.left() - 7.5));
    assert!(approx(cl.btn_give.height(), 30.0));
    assert!(approx(cl.btn_deny.height(), 30.0));
    assert!(approx(cl.btn_approve.height(), 30.0));
    // Exact button widths from the reference (computed styles): the button is
    // padding-x 7.5 + icon 15 + gap 3.75 + the label's px-4 wrapper
    // (text + 2×3.75) + padding-x 7.5 (+ 2×1 border for secondary). The
    // residual (<0.05px) is GPOS-vs-harfrust kerning on the label.
    for (rect, want) in [
        (cl.btn_give, 129.547),
        (cl.btn_deny, 72.156),
        (cl.btn_approve, 91.953),
    ] {
        assert!(
            (rect.width() - want).abs() < 0.05,
            "button width {:.4}, reference {want}",
            rect.width()
        );
    }

    // Card height and scroll extent.
    let card_h = cl.content_h + 2.0 * (1.0 + 22.5);
    assert!(approx(cl.card.height(), card_h));
    assert!(approx(l.content_height, card_h + 45.0));
    assert!(approx(l.max_scroll, (l.content_height - l.main.height()).max(0.0)));

    // Footer without cooldowns: 1px border + 11.25×2 + text.
    let footer_text_h = components::block_height(&l.footer_text.galley, m.body_xs);
    assert!(approx(l.footer.height(), 1.0 + 22.5 + footer_text_h));
    assert!(approx(l.footer_text.pos.y, l.footer.top() + 1.0 + 11.25));
    assert!(approx(l.footer_text.pos.x, 22.5));
    assert!(approx(l.main.bottom(), l.footer.top()));
    assert!(l.cooldown_row.is_none());
}

#[test]
fn cooldown_row_adds_a_status_line_to_the_footer() {
    let mut app = app_with(vec![demo::demo_request()]);
    app.badge.jump(1, 0.0);
    app.cooldowns = vec![
        ("#eng-deploys".to_string(), "excerpt".to_string(), 443),
        ("#general".to_string(), "excerpt".to_string(), 42),
    ];
    let l = layout_for(&app, 720.0, 780.0);
    let row = l.cooldown_row.as_ref().expect("cooldown row");
    assert_eq!(
        row.text_string,
        "Deny cooldowns active: #eng-deploys (8 min left), #general (1 min left)"
    );
    let m = Metrics::new(720.0);
    assert!(approx(
        row.row_top,
        l.footer_text.pos.y + components::block_height(&l.footer_text.galley, m.body_xs) + 3.75
    ));
    assert!(approx(row.dot_x, 22.5));
    assert!(approx(row.text.pos.x, 22.5 + 3.75 + 7.5));
    assert!(approx(row.row_h, 15.0));
    let footer_text_h = components::block_height(&l.footer_text.galley, m.body_xs);
    assert!(approx(
        l.footer.height(),
        1.0 + 22.5 + footer_text_h + 3.75 + 15.0
    ));
}

#[test]
fn feedback_open_adds_textarea_submit_row_and_gap() {
    let mut app = app_with(vec![demo::demo_request()]);
    app.badge.jump(1, 0.0);
    app.feedback_open = true;
    let l = layout_for(&app, 720.0, 780.0);
    let cl = l.card_layout.as_ref().unwrap();

    let ta = cl.ta_box.as_ref().expect("textarea box");
    assert!(approx(ta.min.x, 46.0));
    assert!(approx(ta.width(), 628.0));
    // Empty feedback text: one line → the box's min height.
    assert!(approx(ta.height(), 52.5));
    let submit = cl.submit_btn.as_ref().expect("submit button");
    assert!(approx(submit.right(), 674.0));
    assert!(approx(submit.top(), ta.bottom() + 7.5));
    assert!(approx(submit.height(), 30.0));
    // Buttons row after the actions gap (space-12).
    assert!(approx(cl.btn_give.top(), submit.bottom() + 11.25));
    assert!(cl.error_text.is_none());

    // The card is exactly the feedback-closed card plus the feedback
    // column (52.5 + 7.5 + 30) and one more actions gap (11.25).
    let mut closed = app_with(vec![demo::demo_request()]);
    closed.badge.jump(1, 0.0);
    let l_closed = layout_for(&closed, 720.0, 780.0);
    let h_closed = l_closed.card_layout.as_ref().unwrap().card.height();
    assert!(approx(
        l.card_layout.as_ref().unwrap().card.height(),
        h_closed + 52.5 + 7.5 + 30.0 + 11.25
    ));
}

#[test]
fn submission_error_sits_between_feedback_and_buttons() {
    let mut app = app_with(vec![demo::demo_request()]);
    app.badge.jump(1, 0.0);
    app.feedback_open = true;
    app.submit_error = true;
    let l = layout_for(&app, 720.0, 780.0);
    let cl = l.card_layout.as_ref().unwrap();
    let e = cl.error_text.as_ref().expect("error text");
    let submit = cl.submit_btn.as_ref().unwrap();
    assert!(approx(e.pos.y, submit.bottom() + 11.25));
    assert!(approx(e.pos.x, 46.0));
    assert!(approx(
        cl.btn_give.top(),
        e.pos.y + components::block_height(&e.galley, Metrics::new(720.0).body_xs) + 11.25
    ));

    // Feedback closed + error: error directly above the buttons.
    let mut app2 = app_with(vec![demo::demo_request()]);
    app2.badge.jump(1, 0.0);
    app2.submit_error = true;
    let l2 = layout_for(&app2, 720.0, 780.0);
    let cl2 = l2.card_layout.as_ref().unwrap();
    assert!(cl2.ta_box.is_none());
    let e2 = cl2.error_text.as_ref().unwrap();
    assert!(approx(
        cl2.btn_give.top(),
        e2.pos.y + components::block_height(&e2.galley, Metrics::new(720.0).body_xs) + 11.25
    ));
}

#[test]
fn empty_state_is_centered_and_sized() {
    let app = app_with(Vec::new());
    let l = layout_for(&app, 720.0, 780.0);
    assert!(l.badge.is_none());
    assert!(l.card_layout.is_none());
    let el = l.empty.as_ref().expect("empty state");
    assert!(approx(el.card.min.x, 22.5));
    assert!(approx(el.card.top(), 91.0));
    assert!(approx(el.card.width(), 675.0));
    // Icon: 30px (Icon size l), centered at the content top.
    assert!(approx(el.icon_size, 30.0));
    assert!(approx(el.icon.x, 360.0 - 15.0));
    assert!(approx(el.icon.y, 137.0));
    // Title + sub line boxes are centered on the card's center x (the paint
    // origin is center − width/2, so origin + galley center == 360 exactly).
    assert!(approx(el.title.pos.x + el.title.galley.rect.center().x, 360.0));
    assert!(approx(el.title.pos.y, 137.0 + 30.0 + 11.25));
    assert!(approx(
        components::block_height(&el.title.galley, Metrics::new(720.0).body_strong_m),
        22.5
    ));
    assert!(approx(el.sub.pos.x + el.sub.galley.rect.center().x, 360.0));
    assert!(approx(el.sub.pos.y, el.title.pos.y + 22.5 + 11.25));
    // Card height = content + 2 × (1px border + space-48 padding).
    assert!(approx(el.card.height(), el.content_h + 2.0 * 46.0));
}

#[test]
fn unresolved_destination_shows_only_the_raw_id() {
    let app = app_with(vec![demo::unresolved_request()]);
    let l = layout_for(&app, 720.0, 780.0);
    let cl = l.card_layout.as_ref().unwrap();
    let (_, info_label) = &cl.info_tag;
    assert_eq!(info_label, "U09UNKNOWN99");
    assert!(cl.raw_tag.is_none(), "no raw tag when unresolved");
    assert!(cl.thread_tag.is_none(), "no thread badge without a thread");
}

#[test]
fn layout_scales_with_the_responsive_root_at_1000px() {
    let mut app = app_with(vec![demo::demo_request()]);
    app.badge.jump(1, 0.0);
    let l = layout_for(&app, 1000.0, 780.0);
    let s = 16.0 / 15.0;
    // Header: 16 + 48 + 16 + 1.
    assert!(approx(l.header.height(), 81.0));
    // Card: 952 wide (1000 − 2×24), centered at x=24.
    let cl = l.card_layout.as_ref().unwrap();
    assert!(approx(cl.card.width(), 952.0));
    assert!(approx(cl.card.min.x, 24.0));
    assert!(approx(cl.card.top(), 81.0 + 22.5 * s));
    assert!(approx(cl.content_x, 24.0 + 1.0 + 22.5 * s));
    assert!(approx(cl.cw, 952.0 - 2.0 * (1.0 + 22.5 * s)));
    // Title at the scaled position.
    assert!(approx(l.title_pos.x, 22.5 * s + 20.0 + 7.5 * s));
    assert!(approx(l.title_pos.y, 16.0));
    // Buttons still end at the content's right edge.
    assert!(approx(cl.btn_approve.right(), cl.content_x + cl.cw));
}

#[test]
fn narrow_window_keeps_the_card_centered_and_scrollable() {
    let mut app = app_with(vec![demo::long_request()]);
    app.badge.jump(1, 0.0);
    let l = layout_for(&app, 520.0, 560.0);
    let cl = l.card_layout.as_ref().unwrap();
    assert!(approx(cl.card.width(), 475.0));
    assert!(approx(cl.card.min.x, 22.5));
    assert!(approx(cl.cw, 475.0 - 2.0 * 23.5));
    // The tall card scrolls.
    assert!(l.max_scroll > 0.0);
    assert!(approx(
        l.max_scroll,
        (cl.card.height() + 45.0 - l.main.height()).max(0.0)
    ));
}

#[test]
fn narrow_heading_row_wraps_the_agent_tag_with_per_line_heights() {
    // At 520px the heading row wraps: the agent tag drops below the heading
    // column. CSS flexbox gives each wrapped line the height of the tallest
    // item *in that line* — the tag's line is the tag's height, not the
    // heading column's.
    let mut app = app_with(vec![demo::demo_request()]);
    app.badge.jump(1, 0.0);
    let l = layout_for(&app, 520.0, 560.0);
    let cl = l.card_layout.as_ref().unwrap();
    let (tag_rect, tag_label) = &cl.agent_tag;
    assert_eq!(tag_label, "claude-code");
    // The tag starts a second line at the content's left edge, below the
    // heading column (37.5 + 3.75 + 15) plus the 15px row gap.
    assert!(approx(tag_rect.min.x, 46.0));
    assert!(approx(tag_rect.min.y, 114.5 + 56.25 + 15.0));
    assert!(approx(tag_rect.height(), 20.75));
    // The row is 56.25 + 15 + 20.75 tall (not 2 × 56.25 + 15), so the
    // message label lands at the reference's position.
    assert!(approx(cl.label_msg.pos.y, 114.5 + 92.0 + 18.75));
}

#[test]
fn multi_queue_shows_one_card_and_a_counting_badge() {
    let mut app = app_with(vec![
        demo::demo_request(),
        demo::long_request(),
        demo::third_request(),
    ]);
    app.badge.jump(3, 0.0);
    let l = layout_for(&app, 720.0, 780.0);
    let cl = l.card_layout.as_ref().unwrap();
    assert_eq!(cl.subline_text.split(' ').nth(1), Some("demo-0001"));
    let (_, label) = l.badge.as_ref().unwrap();
    assert_eq!(label, "3 waiting");
}

#[test]
fn badge_mounts_with_the_count_then_springs_between_counts() {
    let mut app = app_with(Vec::new());
    app.update_badge(0.0);
    assert_eq!(app.badge.display(0.0), "0");

    // First request: the badge mounts showing the count immediately (the
    // reference renders `queue.length > 0 && <Tag>` and `useSpring(value)`
    // initializes at the source value — no 0→N animation).
    app.queue.push_back(demo::demo_request());
    app.update_badge(1.0);
    assert!(app.badge_mounted);
    assert_eq!(app.badge.display(1.0), "1");

    // Second request: the mounted badge springs 1 → 2 (continuous motion).
    app.queue.push_back(demo::long_request());
    app.update_badge(2.0);
    assert_eq!(app.badge.display(2.02), "1", "must animate, not jump");
    assert_eq!(app.badge.display(10.0), "2");

    // Queue drains: the badge unmounts and snaps back to zero (invisible).
    app.queue.pop_front();
    app.queue.pop_front();
    app.update_badge(3.0);
    assert!(!app.badge_mounted);
    assert_eq!(app.badge.display(3.0), "0");

    // A later request remounts at the new count — never animates 0 → N.
    app.queue.push_back(demo::third_request());
    app.update_badge(4.0);
    assert_eq!(app.badge.display(4.0), "1");
}

