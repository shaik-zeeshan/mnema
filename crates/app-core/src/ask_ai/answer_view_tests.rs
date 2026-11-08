use super::*;

/// Fold an op stream into blocks using the SAME apply semantics the frontend
/// uses: `AppendProse` appends to the trailing prose block (or creates one);
/// `OpenBlock` pushes a block. Other ops are irrelevant to this parser.
fn apply_ops(ops: &[TurnUpdate]) -> Vec<AnswerBlock> {
    let mut blocks: Vec<AnswerBlock> = Vec::new();
    for op in ops {
        match op {
            TurnUpdate::AppendProse { text } => match blocks.last_mut() {
                Some(AnswerBlock::Prose { markdown }) => markdown.push_str(text),
                _ => blocks.push(AnswerBlock::Prose {
                    markdown: text.clone(),
                }),
            },
            TurnUpdate::OpenBlock { block } => blocks.push(block.clone()),
            _ => {}
        }
    }
    blocks
}

/// Feed the whole text in one delta, then finalize; return (parser, all ops).
fn run_whole(text: &str) -> (AnswerView, Vec<TurnUpdate>) {
    let mut v = AnswerView::new();
    let mut ops = v.push_delta(text);
    ops.extend(v.finalize());
    (v, ops)
}

/// Feed the text one char (one UTF-8 codepoint) at a time, then finalize.
fn run_char_by_char(text: &str) -> (AnswerView, Vec<TurnUpdate>) {
    let mut v = AnswerView::new();
    let mut ops = Vec::new();
    for ch in text.chars() {
        let mut buf = [0u8; 4];
        ops.extend(v.push_delta(ch.encode_utf8(&mut buf)));
    }
    ops.extend(v.finalize());
    (v, ops)
}

const BARS_FENCE: &str = "```mnema-bars\n{\"title\":\"Top apps\",\"bars\":[{\"label\":\"Editor\",\"value\":42,\"sublabel\":\"2h\"},{\"label\":\"Browser\",\"value\":18}]}\n```";

const DOSSIER_FENCE: &str = "```mnema-dossier\n{\"items\":[{\"subject\":\"Alice\",\"statement\":\"ships fast\",\"confidence\":0.8},{\"statement\":\"reviews carefully\"}]}\n```";

const TIMELINE_FENCE: &str = "```mnema-timeline\n{\"title\":\"Day\",\"intervals\":[{\"label\":\"Coding\",\"start\":\"2026-06-13T10:00:00Z\",\"end\":\"2026-06-13T11:00:00Z\",\"app\":\"Editor\"}]}\n```";

// 1. Plain prose only.
#[test]
fn plain_prose_only() {
    let text = "Hello world.\n\nThis is some **markdown** prose.";
    let (v, ops) = run_whole(text);
    assert_eq!(v.blocks().len(), 1);
    match &v.blocks()[0] {
        AnswerBlock::Prose { markdown } => assert_eq!(markdown, text),
        other => panic!("expected prose, got {other:?}"),
    }
    // Concatenation of all AppendProse text equals the input.
    let cat: String = ops
        .iter()
        .filter_map(|o| match o {
            TurnUpdate::AppendProse { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(cat, text);
    // No OpenBlock ops for pure prose.
    assert!(!ops
        .iter()
        .any(|o| matches!(o, TurnUpdate::OpenBlock { .. })));
}

// 2. prose → chart → prose.
#[test]
fn prose_chart_prose() {
    let text = format!("Before.\n\n{BARS_FENCE}\n\nAfter.");
    let (v, ops) = run_whole(&text);
    assert_eq!(v.blocks().len(), 3);
    assert!(matches!(v.blocks()[0], AnswerBlock::Prose { .. }));
    assert!(matches!(v.blocks()[1], AnswerBlock::Bars { .. }));
    assert!(matches!(v.blocks()[2], AnswerBlock::Prose { .. }));

    // OpenBlock{Bars} appears between AppendProse runs.
    let open_idx = ops
        .iter()
        .position(|o| {
            matches!(
                o,
                TurnUpdate::OpenBlock {
                    block: AnswerBlock::Bars { .. }
                }
            )
        })
        .expect("OpenBlock{Bars}");
    assert!(open_idx > 0, "prose before the chart");
    assert!(
        ops[open_idx + 1..]
            .iter()
            .any(|o| matches!(o, TurnUpdate::AppendProse { .. })),
        "prose after the chart"
    );

    // The bars block carries the ported fields.
    if let AnswerBlock::Bars { title, items } = &v.blocks()[1] {
        assert_eq!(title.as_deref(), Some("Top apps"));
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].label, "Editor");
        assert_eq!(items[0].value, 42.0);
        assert_eq!(items[0].sublabel.as_deref(), Some("2h"));
        assert_eq!(items[1].sublabel, None);
    } else {
        panic!("expected bars");
    }
}

// 3. A fence split across deltas yields the same single block.
#[test]
fn split_fence_equivalent_to_whole() {
    let text = format!("Intro.\n\n{BARS_FENCE}\n\nOutro.");
    let (whole, _) = run_whole(&text);
    let (split, _) = run_char_by_char(&text);
    assert_eq!(whole.blocks(), split.blocks());
    // Exactly one Bars block.
    assert_eq!(
        split
            .blocks()
            .iter()
            .filter(|b| matches!(b, AnswerBlock::Bars { .. }))
            .count(),
        1
    );
}

// 4. Malformed JSON inside a CLOSED fence degrades to prose.
#[test]
fn malformed_closed_fence_degrades_to_prose() {
    let fence = "```mnema-bars\n{not valid json}\n```";
    let text = format!("Pre.\n\n{fence}\n\nPost.");
    let (v, _) = run_whole(&text);
    // No Bars block.
    assert!(!v
        .blocks()
        .iter()
        .any(|b| matches!(b, AnswerBlock::Bars { .. })));
    // The raw fence text is present in prose.
    let prose: String = v
        .blocks()
        .iter()
        .filter_map(|b| match b {
            AnswerBlock::Prose { markdown } => Some(markdown.as_str()),
            _ => None,
        })
        .collect();
    assert!(prose.contains("```mnema-bars"));
    assert!(prose.contains("{not valid json}"));
}

// 4b. Closed fence with no valid items also degrades.
#[test]
fn empty_items_fence_degrades_to_prose() {
    let fence = "```mnema-bars\n{\"bars\":[]}\n```";
    let (v, _) = run_whole(fence);
    assert!(!v
        .blocks()
        .iter()
        .any(|b| matches!(b, AnswerBlock::Bars { .. })));
    assert!(matches!(
        v.blocks().first(),
        Some(AnswerBlock::Prose { .. })
    ));
}

// 5. An unterminated fence at finalize() degrades to prose.
#[test]
fn unterminated_fence_degrades_on_finalize() {
    let text = "Lead.\n\n```mnema-bars\n{\"bars\":[{\"label\":\"x\",\"value\":1}]";
    let mut v = AnswerView::new();
    let ops_stream = v.push_delta(text);
    // While still open, no block and the held fence text is NOT yet prose.
    assert!(v
        .blocks()
        .iter()
        .all(|b| !matches!(b, AnswerBlock::Bars { .. })));
    let prose_before: String = v
        .blocks()
        .iter()
        .filter_map(|b| match b {
            AnswerBlock::Prose { markdown } => Some(markdown.as_str()),
            _ => None,
        })
        .collect();
    assert!(!prose_before.contains("mnema-bars"), "held back while open");

    let final_ops = v.finalize();
    // After finalize the raw fence is prose.
    let prose_after: String = v
        .blocks()
        .iter()
        .filter_map(|b| match b {
            AnswerBlock::Prose { markdown } => Some(markdown.as_str()),
            _ => None,
        })
        .collect();
    assert!(prose_after.contains("```mnema-bars"));
    assert!(!v
        .blocks()
        .iter()
        .any(|b| matches!(b, AnswerBlock::Bars { .. })));

    // Op-replay still reconstructs the blocks.
    let mut all = ops_stream;
    all.extend(final_ops);
    assert_eq!(apply_ops(&all), v.blocks());
}

// 6. Op-replay equivalence (the critical invariant).
#[test]
fn op_replay_equivalence_whole_vs_char() {
    let text = format!(
        "Top apps today.\n\n{BARS_FENCE}\n\nAnd findings:\n\n{DOSSIER_FENCE}\n\nTimeline:\n\n{TIMELINE_FENCE}\n\nDone."
    );

    let (whole, whole_ops) = run_whole(&text);
    let (split, split_ops) = run_char_by_char(&text);

    // Same final blocks regardless of chunking.
    assert_eq!(whole.blocks(), split.blocks());

    // Op replay reconstructs the parser's blocks for BOTH feeds.
    assert_eq!(apply_ops(&whole_ops), whole.blocks());
    assert_eq!(apply_ops(&split_ops), split.blocks());
}

// 6b. Op replay for a two-char-at-a-time feed too (boundary stress).
#[test]
fn op_replay_chunked_pairs() {
    let text = format!("A.\n\n{TIMELINE_FENCE}\n\nB.");
    let mut v = AnswerView::new();
    let mut ops = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let chunk: String = chars[i..(i + 2).min(chars.len())].iter().collect();
        ops.extend(v.push_delta(&chunk));
        i += 2;
    }
    ops.extend(v.finalize());
    assert_eq!(apply_ops(&ops), v.blocks());
    assert!(v
        .blocks()
        .iter()
        .any(|b| matches!(b, AnswerBlock::Timeline { .. })));
}

// 7. Dossier happy path with ported field handling.
#[test]
fn dossier_happy_path() {
    let (v, _) = run_whole(DOSSIER_FENCE);
    assert_eq!(v.blocks().len(), 1);
    if let AnswerBlock::Dossier { items } = &v.blocks()[0] {
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].subject.as_deref(), Some("Alice"));
        assert_eq!(items[0].statement, "ships fast");
        assert_eq!(items[0].confidence, 0.8);
        // Missing subject → None; missing confidence → 0.0.
        assert_eq!(items[1].subject, None);
        assert_eq!(items[1].confidence, 0.0);
    } else {
        panic!("expected dossier");
    }
}

// 7b. Dossier confidence clamps to [0,1]; blank statement dropped.
#[test]
fn dossier_clamps_and_drops_blank() {
    let fence = "```mnema-dossier\n{\"items\":[{\"statement\":\"  \",\"confidence\":0.5},{\"statement\":\"hot\",\"confidence\":5.0},{\"statement\":\"cold\",\"confidence\":-2.0}]}\n```";
    let (v, _) = run_whole(fence);
    if let AnswerBlock::Dossier { items } = &v.blocks()[0] {
        assert_eq!(items.len(), 2, "blank-statement item dropped");
        assert_eq!(items[0].confidence, 1.0, "clamped high");
        assert_eq!(items[1].confidence, 0.0, "clamped low");
    } else {
        panic!("expected dossier");
    }
}

// 7c. Timeline happy path; drops items missing label or start.
#[test]
fn timeline_happy_path_and_drops() {
    let fence = "```mnema-timeline\n{\"intervals\":[{\"label\":\"Coding\",\"start\":\"2026-06-13T10:00:00Z\",\"app\":\"Editor\"},{\"start\":\"2026-06-13T11:00:00Z\"},{\"label\":\"Meeting\"}]}\n```";
    let (v, _) = run_whole(fence);
    if let AnswerBlock::Timeline { title, items } = &v.blocks()[0] {
        assert_eq!(*title, None);
        assert_eq!(items.len(), 1, "items missing label or start dropped");
        assert_eq!(items[0].label, "Coding");
        assert_eq!(items[0].start, "2026-06-13T10:00:00Z");
        assert_eq!(items[0].app.as_deref(), Some("Editor"));
        assert_eq!(items[0].end, None);
    } else {
        panic!("expected timeline");
    }
}

// Bars value coercion from a numeric string (matches frontend Number()).
#[test]
fn bars_value_coerces_numeric_string() {
    let fence = "```mnema-bars\n{\"bars\":[{\"label\":\"x\",\"value\":\"3.5\"},{\"label\":\"bad\",\"value\":\"oops\"}]}\n```";
    let (v, _) = run_whole(fence);
    if let AnswerBlock::Bars { items, .. } = &v.blocks()[0] {
        assert_eq!(items.len(), 1, "non-numeric value dropped");
        assert_eq!(items[0].value, 3.5);
    } else {
        panic!("expected bars");
    }
}

// An ordinary (non-mnema) code fence stays prose and is NOT intercepted.
#[test]
fn ordinary_code_fence_is_prose() {
    let text = "Here:\n\n```rust\nfn main() {}\n```\n\nDone.";
    let (v, _) = run_whole(text);
    assert!(v
        .blocks()
        .iter()
        .all(|b| matches!(b, AnswerBlock::Prose { .. })));
    let prose: String = v
        .blocks()
        .iter()
        .filter_map(|b| match b {
            AnswerBlock::Prose { markdown } => Some(markdown.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(prose, text);
}

// A fence whose info string merely CONTAINS but doesn't START with mnema-* is
// ordinary prose.
#[test]
fn near_miss_info_string_is_prose() {
    let text = "```not-mnema-bars\n{}\n```";
    let (v, _) = run_whole(text);
    assert!(v
        .blocks()
        .iter()
        .all(|b| matches!(b, AnswerBlock::Prose { .. })));
}

// parse_answer_to_blocks: a whole answer (prose + a valid mnema-bars fence)
// parses to [Prose, Bars] — the same result the live stream produces, which
// is what the get_conversation command uses to backfill legacy turns.
#[test]
fn parse_answer_to_blocks_prose_then_bars() {
    let answer = format!("Top apps today.\n\n{BARS_FENCE}");
    let blocks = parse_answer_to_blocks(&answer);
    assert_eq!(blocks.len(), 2);
    assert!(matches!(blocks[0], AnswerBlock::Prose { .. }));
    assert!(matches!(blocks[1], AnswerBlock::Bars { .. }));
}

// An empty answer parses to no blocks (the command guards on this too,
// but the parser itself must not invent a block).
#[test]
fn parse_answer_to_blocks_empty_is_no_blocks() {
    assert!(parse_answer_to_blocks("").is_empty());
}

// Incremental close-scan: a long single-line (newline-free) JSON body fed one
// char at a time still finds the close exactly once and matches the whole-feed
// result. Pins that the resume-offset optimization preserves parsing behavior
// for the body shape (one long line) that drove the O(n²) hot path.
#[test]
fn long_single_line_body_char_by_char() {
    let bars: String = (0..200)
        .map(|i| format!("{{\"label\":\"item {i}\",\"value\":{i}}}"))
        .collect::<Vec<_>>()
        .join(",");
    let fence = format!("```mnema-bars\n{{\"bars\":[{bars}]}}\n```");
    let text = format!("Lead.\n\n{fence}\n\nTail.");
    let (whole, _) = run_whole(&text);
    let (split, split_ops) = run_char_by_char(&text);
    assert_eq!(whole.blocks(), split.blocks());
    assert_eq!(
        split
            .blocks()
            .iter()
            .filter(|b| matches!(b, AnswerBlock::Bars { .. }))
            .count(),
        1
    );
    assert_eq!(apply_ops(&split_ops), split.blocks());
}

// Incremental close-scan: the closing ` ``` ` arriving split across deltas
// (newline, then one backtick at a time) must still be detected — the resume
// offset has to hold back a growing backtick prefix at a line start.
#[test]
fn close_fence_split_backtick_by_backtick() {
    let mut v = AnswerView::new();
    let mut ops = v.push_delta("```mnema-bars\n{\"bars\":[{\"label\":\"x\",\"value\":1}]}");
    ops.extend(v.push_delta("\n"));
    ops.extend(v.push_delta("`"));
    ops.extend(v.push_delta("`"));
    ops.extend(v.push_delta("`"));
    ops.extend(v.finalize());
    assert!(v
        .blocks()
        .iter()
        .any(|b| matches!(b, AnswerBlock::Bars { .. })));
    assert_eq!(apply_ops(&ops), v.blocks());
}

// Trailing-backtick hold-back: a delta ending mid-opener must not leak prose
// that later becomes a fence.
#[test]
fn split_at_backtick_boundary() {
    let mut v = AnswerView::new();
    let mut ops = v.push_delta("Intro\n``");
    ops.extend(v.push_delta("`mnema-bars\n{\"bars\":[{\"label\":\"x\",\"value\":1}]}\n```"));
    ops.extend(v.finalize());
    // Exactly one Bars block, prose "Intro\n" preserved.
    assert!(v
        .blocks()
        .iter()
        .any(|b| matches!(b, AnswerBlock::Bars { .. })));
    assert_eq!(apply_ops(&ops), v.blocks());
}
