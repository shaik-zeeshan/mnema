use super::*;

// ── LiveTurn store / versioned transport (Slice 4) ───────────────────────
// The static LiveTurn map persists across tests, so each test uses a UNIQUE
// conversation id and cleans up after itself with `remove_live_turn_if_owner`.

use capture_types::{AnswerBlock, BarsItem, ToolActivityEntry};

#[test]
fn live_update_versions_are_monotonic() {
    let conv = "live-monotonic-conv";
    let token = next_turn_token();
    register_live_turn(conv, token, fresh_view(0));

    let v1 = apply_live_update(
        conv,
        token,
        TurnUpdate::Phase {
            phase: "streaming".to_string(),
        },
    )
    .expect("owned apply succeeds");
    let v2 = apply_live_update(
        conv,
        token,
        TurnUpdate::AppendProse {
            text: "hi".to_string(),
        },
    )
    .expect("owned apply succeeds");
    let v3 = apply_live_update(
        conv,
        token,
        TurnUpdate::AppendProse {
            text: " there".to_string(),
        },
    )
    .expect("owned apply succeeds");

    assert_eq!(v1.0, 1);
    assert_eq!(v2.0, 2);
    assert_eq!(v3.0, 3);

    let snapshot = snapshot_live_turn(conv).expect("snapshot present");
    assert_eq!(snapshot.version, 3);
    assert_eq!(snapshot.conversation_id, conv);

    remove_live_turn_if_owner(conv, token);
    assert!(snapshot_live_turn(conv).is_none(), "cleaned up");
}

#[test]
fn displaced_turn_cannot_apply_or_evict() {
    let conv = "live-displace-conv";
    let token_a = next_turn_token();
    register_live_turn(conv, token_a, fresh_view(0));

    // A newer turn for the SAME conversation overwrites the entry.
    let token_b = next_turn_token();
    register_live_turn(conv, token_b, fresh_view(1));

    // The displaced turn A can no longer apply (returns None, emits nothing).
    assert!(
        apply_live_update(
            conv,
            token_a,
            TurnUpdate::AppendProse {
                text: "stale".to_string()
            },
        )
        .is_none(),
        "displaced turn must not apply"
    );

    // The owning turn B still applies.
    let applied = apply_live_update(
        conv,
        token_b,
        TurnUpdate::AppendProse {
            text: "fresh".to_string(),
        },
    );
    assert!(applied.is_some(), "owning turn applies");
    assert_eq!(
        applied.unwrap().0,
        1,
        "B's first applied update is version 1"
    );

    // The displaced turn A removing must NOT evict B's entry.
    remove_live_turn_if_owner(conv, token_a);
    let snapshot = snapshot_live_turn(conv).expect("B's entry survives A's teardown");
    assert_eq!(snapshot.view.turn_index, 1, "the surviving entry is B's");

    // B tears down its own entry cleanly.
    remove_live_turn_if_owner(conv, token_b);
    assert!(snapshot_live_turn(conv).is_none(), "cleaned up");
}

#[test]
fn snapshot_reflects_applied_ops() {
    let conv = "live-snapshot-ops-conv";
    let token = next_turn_token();
    register_live_turn(conv, token, fresh_view(0));

    apply_live_update(
        conv,
        token,
        TurnUpdate::AppendProse {
            text: "Top apps:".to_string(),
        },
    );
    let bars = AnswerBlock::Bars {
        title: Some("Top apps".to_string()),
        items: vec![BarsItem {
            label: "Editor".to_string(),
            value: 42.0,
            sublabel: None,
        }],
    };
    apply_live_update(
        conv,
        token,
        TurnUpdate::OpenBlock {
            block: bars.clone(),
        },
    );
    apply_live_update(
        conv,
        token,
        TurnUpdate::Reasoning {
            text: "thinking…".to_string(),
        },
    );

    let snapshot = snapshot_live_turn(conv).expect("snapshot present");
    assert_eq!(snapshot.view.blocks.len(), 2);
    assert!(matches!(snapshot.view.blocks[0], AnswerBlock::Prose { .. }));
    assert_eq!(snapshot.view.blocks[1], bars);
    assert_eq!(snapshot.view.reasoning.as_deref(), Some("thinking…"));

    remove_live_turn_if_owner(conv, token);
}

#[test]
fn snapshot_equals_op_replay_onto_fresh_view() {
    let conv = "live-replay-equiv-conv";
    let token = next_turn_token();
    register_live_turn(conv, token, fresh_view(0));

    // A representative op sequence covering every reducer arm relevant to a
    // streaming turn.
    let updates = vec![
        TurnUpdate::Phase {
            phase: "streaming".to_string(),
        },
        TurnUpdate::AppendProse {
            text: "Hello ".to_string(),
        },
        TurnUpdate::AppendProse {
            text: "world.".to_string(),
        },
        TurnUpdate::OpenBlock {
            block: AnswerBlock::Bars {
                title: None,
                items: vec![BarsItem {
                    label: "x".to_string(),
                    value: 1.0,
                    sublabel: None,
                }],
            },
        },
        TurnUpdate::AppendProse {
            text: "After.".to_string(),
        },
        TurnUpdate::Reasoning {
            text: "hmm".to_string(),
        },
        TurnUpdate::ToolActivity {
            entry: ToolActivityEntry {
                kind: "search".to_string(),
                label: "Searching".to_string(),
                app: None,
                app_icon_path: None,
            },
        },
        TurnUpdate::LiveActivity {
            entry: Some(ToolActivityEntry {
                kind: "search".to_string(),
                label: "Searching".to_string(),
                app: None,
                app_icon_path: None,
            }),
        },
        TurnUpdate::Sources {
            sources: serde_json::json!([{ "kind": "frame" }]),
        },
        TurnUpdate::Engine {
            provider: "openai-2".to_string(),
            model: "gpt-5".to_string(),
        },
        TurnUpdate::Done,
    ];

    // Collect the `update`s returned by applying each through the live map.
    let mut returned_ops: Vec<TurnUpdate> = Vec::new();
    for update in updates {
        let (_, _, applied) = apply_live_update(conv, token, update).expect("owned apply");
        returned_ops.push(applied);
    }

    // Fold the returned op stream onto a FRESH view (same initial shape) via
    // the pure reducer.
    let mut folded = fresh_view(0);
    for op in &returned_ops {
        apply_update_to_view(&mut folded, op);
    }

    let snapshot = snapshot_live_turn(conv).expect("snapshot present");
    // Applying the op stream from version 0 == the snapshot at that version.
    assert_eq!(folded, snapshot.view);
    // The engine op lands in the reattach snapshot, not just equally nowhere.
    assert_eq!(snapshot.view.provider.as_deref(), Some("openai-2"));
    assert_eq!(snapshot.view.model.as_deref(), Some("gpt-5"));

    remove_live_turn_if_owner(conv, token);
}

#[test]
fn append_prose_reducer_matches_parser_coalescing() {
    // Feed the SAME text through `AnswerView::push_delta` (the parser) and
    // through `apply_update_to_view` (the reducer): both must coalesce prose
    // the same way — appending to a trailing prose block vs. starting a new one
    // after a typed block.
    let text =
        "Before.\n\n```mnema-bars\n{\"bars\":[{\"label\":\"x\",\"value\":1}]}\n```\n\nAfter.";

    let mut parser = AnswerView::new();
    let mut ops = parser.push_delta(text);
    ops.extend(parser.finalize());

    let mut view = fresh_view(0);
    for op in &ops {
        apply_update_to_view(&mut view, op);
    }

    // The reducer-folded blocks match the parser's own blocks.
    assert_eq!(view.blocks, parser.blocks());
    // Prose · Bars · Prose.
    assert_eq!(view.blocks.len(), 3);
    assert!(matches!(view.blocks[0], AnswerBlock::Prose { .. }));
    assert!(matches!(view.blocks[1], AnswerBlock::Bars { .. }));
    assert!(matches!(view.blocks[2], AnswerBlock::Prose { .. }));
}
