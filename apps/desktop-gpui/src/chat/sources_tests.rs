//! Layer 2 for an answer's sources: chip thumbnails, the frame peek with its
//! lazy OCR, and the audio receipt, all through `FixtureSources` (+ the audio
//! drawer's fixture backend and fake player). Each covers its loaded, removed
//! and late-answer paths.

use super::peek::PeekOps as _;
use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::base::Root;
use gpui_kit::prelude::*;
use gpui_kit::test::{TestQueryExt, TestWindowExt};
use gpui_kit::{Entity, KeyBinding, TestAppContext, WindowHandle};
use serde_json::json;

use super::clip::ClipLoad;
use super::fixture::FixtureSources;
use super::peek::FrameLoad;
use super::tests::open;
use super::{Chat, ChatTurn, Pane};
use crate::timeline::audio::fixture::{FakePlayer, FixtureAudio};
use crate::timeline::audio::Deps;
use crate::timeline::model::OcrBox;
use crate::timeline::OcrStatus;
use crate::ui::nid;

const SEG: i64 = 41;
const CLIP_START: i64 = 1_793_782_821_000; // 2026-11-04T09:00:21Z

fn ocr_box(text: &str) -> OcrBox {
    OcrBox {
        x: 0.1,
        y: 0.1,
        w: 0.3,
        h: 0.05,
        text: text.into(),
        confidence: 0.9,
    }
}

fn fixture() -> FixtureSources {
    FixtureSources {
        thumbs: [(1, Some(PathBuf::from("/nonexistent/t1.jpg"))), (2, None)].into(),
        frames: [(1, (Some(1600), Some(1000))), (2, (Some(800), Some(600)))].into(),
        previews: [(1, PathBuf::from("/nonexistent/f1.jpg"))].into(),
        ocr: [(1, vec![ocr_box("Ship it"), ocr_box("Review")])].into(),
        strip: vec![
            (10, CLIP_START),
            (11, CLIP_START + 20_000),
            (12, CLIP_START + 40_000),
        ],
        ..Default::default()
    }
}

/// A done answer citing frames 1 and 2 and clip `segment`, served by `src`.
fn setup(
    cx: &mut TestAppContext,
    src: FixtureSources,
    segment: i64,
) -> (
    WindowHandle<Root>,
    Entity<Chat>,
    Arc<FixtureSources>,
    FakePlayer,
) {
    let (w, chat) = open(cx);
    let src = Arc::new(src);
    let player = FakePlayer::new(50.);
    let audio = FixtureAudio::from_map(json!({ "get_audio_segment": {
        "id": SEG, "sourceKind": "microphone", "sourceSessionId": "s",
        "segmentIndex": 0, "filePath": "/nonexistent/mic.m4a",
        "startedAt": "2026-11-04T09:00:21.000Z", "endedAt": "2026-11-04T09:01:11.000Z"
    }}));
    chat.update(cx, |c, cx| {
        c.sources = src.clone();
        c.audio = Deps {
            backend: Arc::new(audio),
            open_player: player.factory(),
        };
        c.active = Some("c1".into());
        c.pane = Pane::Thread;
        let mut t = ChatTurn::new(0, "What did I ship?", "done");
        t.view.sources = json!([
            { "kind": "frame", "frameId": 1, "appName": "Safari", "windowTitle": "Docs",
              "startedAt": "2026-11-04T09:00:21Z" },
            { "kind": "frame", "frameId": 2, "appName": "Notes", "startedAt": "2026-11-04T09:00:30Z" },
            { "kind": "audio", "audioSegmentId": segment, "sourceKind": "microphone",
              "startedAt": "2026-11-04T09:00:21.000Z", "endedAt": "2026-11-04T09:01:11.000Z" }
        ]);
        c.turns.push(t);
        cx.notify();
    });
    (w, chat, src, player)
}

fn click(cx: &mut TestAppContext, w: WindowHandle<Root>, id: gpui_kit::ElementId) {
    cx.update_window(w.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn draw(cx: &mut TestAppContext, w: WindowHandle<Root>, chat: &Entity<Chat>) {
    chat.update(cx, |_, cx| cx.notify());
    cx.update_window(w.into(), |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
}

fn count(src: &FixtureSources, prefix: &str) -> usize {
    src.calls().iter().filter(|c| c.starts_with(prefix)).count()
}

#[gpui_kit::test]
fn thumbnails_load_once_per_frame_and_mark_a_frame_with_no_preview(cx: &mut TestAppContext) {
    let (w, chat, src, _) = setup(cx, fixture(), SEG);
    draw(cx, w, &chat);
    let thumbs = chat.read_with(cx, |c, _| c.thumbs.clone());
    assert_eq!(
        thumbs.get(&1),
        Some(&Some(PathBuf::from("/nonexistent/t1.jpg")))
    );
    assert_eq!(thumbs.get(&2), Some(&None), "no preview any more");
    draw(cx, w, &chat);
    assert_eq!(
        src.calls(),
        ["thumbs [1, 2]"],
        "asked once, both frames together"
    );
}

#[gpui_kit::test]
fn a_late_thumbnail_answer_for_a_chat_no_longer_open_is_dropped(cx: &mut TestAppContext) {
    let (_, chat, src, _) = setup(cx, fixture(), SEG);
    chat.update(cx, |c, cx| {
        c.request_thumbs(cx);
        c.reset(Some("c2".into()), cx);
    });
    cx.run_until_parked();
    assert_eq!(count(&src, "thumbs"), 1, "the call went out");
    assert!(chat.read_with(cx, |c, _| c.thumbs.is_empty()));
}

#[gpui_kit::test]
fn the_peek_loads_the_frame_then_its_ocr_only_when_asked(cx: &mut TestAppContext) {
    let (w, chat, src, _) = setup(cx, fixture(), SEG);
    click(cx, w, nid("chat-src-0-f", 0));
    let (frame, image) = chat.read_with(cx, |c, _| {
        (c.peek_data.frame.clone(), c.peek_data.image.clone())
    });
    assert_eq!(
        frame,
        FrameLoad::Ready {
            w: Some(1600),
            h: Some(1000)
        }
    );
    assert_eq!(image, Some(PathBuf::from("/nonexistent/f1.jpg")));
    assert_eq!(count(&src, "ocr"), 0, "OCR is lazy");

    click(cx, w, "chat-frame-ocr".into());
    let (visible, status, n) = chat.read_with(cx, |c, _| {
        let o = &c.peek_data.ocr;
        (o.visible, o.status, o.boxes.len())
    });
    assert_eq!((visible, status, n), (true, OcrStatus::Success, 2));
    draw(cx, w, &chat);
    assert_eq!(super::peek::ocr_count(2), "2");
    assert_eq!(super::peek::ocr_count(2001), "2000+");
    cx.update_window(w.into(), |_, window, _| {
        assert!(
            window.try_find("ocr-overlay").is_some(),
            "boxes over the hero"
        );
    })
    .unwrap();

    click(cx, w, "chat-frame-ocr".into());
    assert!(!chat.read_with(cx, |c, _| c.peek_data.ocr.visible));
    click(cx, w, "chat-frame-ocr".into());
    assert!(chat.read_with(cx, |c, _| c.peek_data.ocr.visible));
    assert_eq!(count(&src, "ocr"), 1, "shown again without a reload");
}

fn advance(cx: &mut TestAppContext, d: std::time::Duration) {
    cx.executor().advance_clock(d);
    cx.run_until_parked();
}

/// Opens the peek's OCR over a job that answers "still running" `pending`
/// times, then re-checks every 750 ms up to 30 s on the test clock.
fn peek_ocr_after(cx: &mut TestAppContext, pending: u32) -> (Entity<Chat>, Arc<FixtureSources>) {
    use crate::timeline::data::{OCR_POLL, OCR_WAIT};
    let (w, chat, src, _) = setup(cx, fixture(), SEG);
    *src.ocr_pending.lock().unwrap() = pending;
    click(cx, w, nid("chat-src-0-f", 0));
    click(cx, w, "chat-frame-ocr".into());
    for _ in 0..(OCR_WAIT.as_millis() / OCR_POLL.as_millis()) {
        advance(cx, OCR_POLL);
    }
    let status = chat.read_with(cx, |c, _| c.peek_data.ocr.status);
    assert_eq!(status, OcrStatus::Running, "30 s");
    assert_eq!(count(&src, "recheck"), 40);
    advance(cx, OCR_POLL);
    (chat, src)
}

/// The peek's OCR waits out a running job like the Timeline's (a timer, not a
/// sleeping thread), on both sides of the 30 s cap.
#[gpui_kit::test]
fn a_running_peek_ocr_job_is_rechecked_on_a_timer_up_to_30s(cx: &mut TestAppContext) {
    use crate::timeline::data::OCR_STILL_RUNNING;
    // The first look and 40 re-checks (0.75 … 30 s) are pending; 30.75 s lands.
    let (chat, _) = peek_ocr_after(cx, 41);
    chat.read_with(cx, |c, _| {
        assert_eq!(c.peek_data.ocr.status, OcrStatus::Success);
        assert_eq!(c.peek_data.ocr.boxes.len(), 2);
    });

    let (chat, _) = peek_ocr_after(cx, 42);
    chat.read_with(cx, |c, _| {
        assert_eq!(c.peek_data.ocr.status, OcrStatus::Error);
        assert_eq!(c.peek_data.ocr.error.as_deref(), Some(OCR_STILL_RUNNING));
    });
}

/// Closing the peek drops the pending OCR's re-checks with it.
#[gpui_kit::test]
fn closing_the_peek_stops_the_ocr_rechecks(cx: &mut TestAppContext) {
    use crate::timeline::data::OCR_POLL;
    let (w, chat, src, _) = setup(cx, fixture(), SEG);
    *src.ocr_pending.lock().unwrap() = u32::MAX;
    click(cx, w, nid("chat-src-0-f", 0));
    click(cx, w, "chat-frame-ocr".into());
    advance(cx, OCR_POLL);
    assert_eq!(count(&src, "recheck"), 1);
    chat.update(cx, |c, cx| c.close_peek(cx));
    advance(cx, OCR_POLL * 4);
    assert_eq!(
        count(&src, "recheck"),
        1,
        "no re-check after the peek closed"
    );
}

#[gpui_kit::test]
fn a_removed_or_unreadable_frame_says_so_and_offers_no_ocr(cx: &mut TestAppContext) {
    let mut f = fixture();
    f.frames.remove(&2);
    let (w, chat, src, _) = setup(cx, f, SEG);
    click(cx, w, nid("chat-src-0-f", 1));
    assert_eq!(
        chat.read_with(cx, |c, _| c.peek_data.frame.clone()),
        FrameLoad::Gone
    );
    draw(cx, w, &chat);
    cx.update_window(w.into(), |_, window, _| {
        assert!(window.try_find_by_label("Frame unavailable").is_some());
    })
    .unwrap();
    chat.update(cx, |c, cx| c.toggle_peek_ocr(cx));
    cx.run_until_parked();
    assert_eq!(count(&src, "ocr"), 0);

    let mut f = fixture();
    f.frame_error = Some("disk".into());
    let (w, chat, _, _) = setup(cx, f, SEG);
    click(cx, w, nid("chat-src-0-f", 0));
    assert_eq!(
        chat.read_with(cx, |c, _| c.peek_data.frame.clone()),
        FrameLoad::Failed("disk".into())
    );
}

#[gpui_kit::test]
fn a_late_peek_answer_never_lands_on_a_closed_or_other_frame(cx: &mut TestAppContext) {
    let (w, chat, _, _) = setup(cx, fixture(), SEG);
    cx.update_window(w.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(nid("chat-src-0-f", 0), cx);
    })
    .unwrap();
    chat.update(cx, |c, cx| c.close_peek(cx));
    cx.run_until_parked();
    let (peek, frame) = chat.read_with(cx, |c, _| (c.peek.is_some(), c.peek_data.frame.clone()));
    assert_eq!((peek, frame), (false, FrameLoad::Loading));

    cx.update_window(w.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(nid("chat-src-0-f", 0), cx);
        // The modal now covers the chips: open frame 2 the way its chip does.
        chat.update(cx, |c, cx| {
            let two = super::model::sources(&c.turns[0].view.sources)[1].clone();
            c.peek_frame(two, window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    let (id, frame) = chat.read_with(cx, |c, _| {
        (
            c.peek.as_ref().and_then(|s| s.frame_id),
            c.peek_data.frame.clone(),
        )
    });
    assert_eq!(id, Some(2));
    assert_eq!(
        frame,
        FrameLoad::Ready {
            w: Some(800),
            h: Some(600)
        },
        "frame 1's answer didn't land on frame 2"
    );
}

#[gpui_kit::test]
fn esc_closes_the_peek(cx: &mut TestAppContext) {
    let (w, chat, _, _) = setup(cx, fixture(), SEG);
    click(cx, w, nid("chat-src-0-f", 0));
    draw(cx, w, &chat);
    cx.simulate_keystrokes(w.into(), "escape");
    cx.run_until_parked();
    assert!(chat.read_with(cx, |c, _| c.peek.is_none()));
}

#[gpui_kit::test]
fn an_audio_chip_opens_its_receipt_that_plays_with_the_strip_on_its_clock(cx: &mut TestAppContext) {
    let (w, chat, src, player) = setup(cx, fixture(), SEG);
    cx.update_window(w.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(nid("chat-src-0-a", 0), cx);
    })
    .unwrap();
    assert_eq!(
        chat.read_with(cx, |c, _| c.clip.as_ref().map(|c| c.load.clone())),
        Some(ClipLoad::Loading)
    );
    cx.run_until_parked();
    let (load, playable, frames) = chat.read_with(cx, |c, _| {
        let c = c.clip.as_ref().unwrap();
        (c.load.clone(), c.transport.playable(), c.strip.len())
    });
    assert_eq!((load, playable, frames), (ClipLoad::Ready, true, 3));
    assert!(src
        .calls()
        .contains(&format!("frames {CLIP_START}..{}", CLIP_START + 50_000)));
    assert!(
        count(&src, "preview") > 0,
        "the strip's previews are fetched"
    );

    click(cx, w, "chat-clip-play".into());
    assert!(player.state().1, "playing");
    player.advance(25.);
    draw(cx, w, &chat);
    let (at, index) = chat.read_with(cx, |c, _| {
        let c = c.clip.as_ref().unwrap();
        (c.transport.current, c.strip.index)
    });
    assert_eq!(
        (at, index),
        (25., 1),
        "the frame captured 20 s in shows at 25 s"
    );

    click(cx, w, "chat-clip-play".into());
    assert!(!player.state().1, "paused");
    click(cx, w, "chat-clip-play".into());
    click(cx, w, nid("chat-src-0-a", 0));
    assert!(
        chat.read_with(cx, |c, _| c.clip.is_none()),
        "a second click closes it"
    );
    assert!(!player.state().1, "and stops the sound");
}

#[gpui_kit::test]
fn esc_closes_the_receipt(cx: &mut TestAppContext) {
    let (w, chat, _, _) = setup(cx, fixture(), SEG);
    // The shell binds Esc to CloseOverlay; this window has no shell.
    cx.update(|cx| cx.bind_keys([KeyBinding::new("escape", crate::keymap::CloseOverlay, None)]));
    click(cx, w, nid("chat-src-0-a", 0));
    draw(cx, w, &chat);
    cx.simulate_keystrokes(w.into(), "escape");
    cx.run_until_parked();
    assert!(chat.read_with(cx, |c, _| c.clip.is_none()));
}

#[gpui_kit::test]
fn a_removed_clip_says_so_and_a_late_clip_answer_is_dropped(cx: &mut TestAppContext) {
    let (w, chat, _, _) = setup(cx, fixture(), SEG + 1);
    click(cx, w, nid("chat-src-0-a", 0));
    assert_eq!(
        chat.read_with(cx, |c, _| c.clip.as_ref().map(|c| c.load.clone())),
        Some(ClipLoad::Gone)
    );
    draw(cx, w, &chat);
    cx.update_window(w.into(), |_, window, _| {
        assert!(window.try_find_by_label("Clip expired").is_some());
    })
    .unwrap();

    let (w, chat, _, player) = setup(cx, fixture(), SEG);
    cx.update_window(w.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(nid("chat-src-0-a", 0), cx);
    })
    .unwrap();
    chat.update(cx, |c, cx| c.reset(Some("c2".into()), cx));
    cx.run_until_parked();
    assert!(
        chat.read_with(cx, |c, _| c.clip.is_none()),
        "never reopened"
    );
    assert!(!player.state().1);
}

fn tool(app: &str, path: Option<&str>) -> capture_types::ToolActivityEntry {
    capture_types::ToolActivityEntry {
        kind: "search".into(),
        label: "Searched your captures".into(),
        app: Some(app.into()),
        app_icon_path: path.map(str::to_string),
    }
}

fn lane(label: &str, start: &str, app: Option<&str>) -> capture_types::TimelineItem {
    capture_types::TimelineItem {
        label: label.into(),
        start: start.into(),
        end: None,
        app: app.map(str::to_string),
        category: Some("research".into()),
    }
}

/// Tool rows (steps open) and a timeline block naming Safari, Notes, Figma
/// and an app the backend already resolved.
fn with_apps(
    cx: &mut TestAppContext,
    icons: &[(&str, &str)],
) -> (WindowHandle<Root>, Entity<Chat>, Arc<FixtureSources>) {
    let src = FixtureSources {
        icons: icons
            .iter()
            .map(|(a, p)| (a.to_string(), PathBuf::from(p)))
            .collect(),
        ..Default::default()
    };
    let (w, chat, src, _) = setup(cx, src, SEG);
    chat.update(cx, |c, cx| {
        let t = &mut c.turns[0];
        t.view.tool_activities = vec![
            tool("Safari", None),
            tool(" safari", None),
            tool("Notes", None),
            tool("Zed", Some("/given/zed.png")),
        ];
        t.view.blocks = vec![capture_types::AnswerBlock::Timeline {
            title: Some("Tuesday morning".into()),
            items: vec![
                lane("Read the notes", "9:00 AM", Some("Safari (Release notes)")),
                lane("Design review", "10:30 AM", Some("Figma")),
                lane("Walk", "12:00 PM", None),
            ],
        }];
        c.steps_open.insert(0);
        cx.notify();
    });
    (w, chat, src)
}

#[gpui_kit::test]
fn app_icons_are_asked_once_per_app_in_one_batch(cx: &mut TestAppContext) {
    let (w, chat, src) = with_apps(cx, &[("Safari", "/nonexistent/safari.png")]);
    draw(cx, w, &chat);
    draw(cx, w, &chat);
    let asked: Vec<_> = src
        .calls()
        .into_iter()
        .filter(|c| c.starts_with("icons"))
        .collect();
    // Case and spacing don't make a new app; the backend's own path isn't re-asked.
    assert_eq!(asked, [r#"icons ["Safari", "Notes", "Figma"]"#]);
    chat.read_with(cx, |c, _| {
        assert_eq!(
            c.icon_for("SAFARI "),
            Some(PathBuf::from("/nonexistent/safari.png"))
        );
        // A miss stays a miss (the initial shows) and is never asked again.
        assert_eq!(c.icon_for("Notes"), None);
    });
    chat.update(cx, |c, cx| c.reset(Some("c2".into()), cx));
    draw(cx, w, &chat);
    assert_eq!(
        count(&src, "icons"),
        1,
        "icons are facts about apps, kept across chats"
    );
}

#[gpui_kit::test]
fn the_timeline_block_renders_its_intervals_on_the_rail(cx: &mut TestAppContext) {
    let (w, chat, _) = with_apps(cx, &[]);
    draw(cx, w, &chat);
    cx.update_window(w.into(), |_, window, _| {
        let rail = window.find("chat-timeline-0-0");
        assert_eq!(rail.label(), Some("Tuesday morning"));
        let rows = window.find_all_by_role(gpui_kit::Role::ListItem);
        let labels: Vec<_> = rows
            .iter()
            .filter_map(|r| r.label().map(str::to_string))
            .filter(|l| l.contains("AM") || l.contains("PM"))
            .collect();
        assert_eq!(
            labels,
            [
                "9:00 AM, Read the notes, Safari",
                "10:30 AM, Design review, Figma",
                "12:00 PM, Walk"
            ]
        );
    })
    .unwrap();
}

/// `FrameDetailModal`'s "copy image": copying…, then "copy failed" when the
/// preview can't be read (or "copied"), back to "copy image" after 1.6 s.
#[gpui_kit::test]
fn the_peek_copies_its_image_and_says_how_it_went(cx: &mut TestAppContext) {
    use super::peek::{CopyState, COPY_REVERT};
    let (w, chat, _, _) = setup(cx, fixture(), SEG);
    click(cx, w, nid("chat-src-0-f", 0));
    click(cx, w, "chat-frame-copy".into());
    assert_eq!(
        chat.read_with(cx, |c, _| c.peek_data.copy),
        CopyState::Failed,
        "the fixture's preview file doesn't exist"
    );
    cx.executor()
        .advance_clock(COPY_REVERT - std::time::Duration::from_millis(1));
    cx.run_until_parked();
    assert_eq!(
        chat.read_with(cx, |c, _| c.peek_data.copy),
        CopyState::Failed
    );
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(1));
    cx.run_until_parked();
    assert_eq!(chat.read_with(cx, |c, _| c.peek_data.copy), CopyState::Idle);
}

/// `FrameDetailModal`'s hero badge ("2 redactions applied") and the meta row's
/// app avatar: the app's icon from `resolve_app_icons`, by the frame's bundle id.
#[gpui_kit::test]
fn the_peek_shows_its_redactions_and_the_app_icon(cx: &mut TestAppContext) {
    let mut f = fixture();
    f.redactions = [(1, 2)].into();
    f.bundles = [(1, "com.apple.Safari".to_string())].into();
    f.icons = [(
        "com.apple.Safari".to_string(),
        PathBuf::from("/nonexistent/safari.png"),
    )]
    .into();
    let (w, chat, _, _) = setup(cx, f, SEG);
    click(cx, w, nid("chat-src-0-f", 0));
    chat.read_with(cx, |c, _| {
        assert_eq!(c.peek_data.redactions, 2);
        assert_eq!(c.peek_data.bundle.as_deref(), Some("com.apple.Safari"));
        assert_eq!(
            c.peek_data.icon,
            Some(PathBuf::from("/nonexistent/safari.png"))
        );
    });
    draw(cx, w, &chat);
    cx.update_window(w.into(), |_, window, _| {
        assert!(window.try_find("chat-frame-redactions").is_some());
        assert!(window.try_find("chat-frame-app").is_some());
    })
    .unwrap();

    // A frame with nothing redacted has no badge.
    chat.update(cx, |c, cx| c.close_peek(cx));
    click(cx, w, nid("chat-src-0-f", 1));
    assert_eq!(chat.read_with(cx, |c, _| c.peek_data.redactions), 0);
    draw(cx, w, &chat);
    cx.update_window(w.into(), |_, window, _| {
        assert!(window.try_find("chat-frame-redactions").is_none());
    })
    .unwrap();
}
