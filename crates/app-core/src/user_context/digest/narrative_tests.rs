use super::*;

/// An engine that ignores the 40-word ask: whole sentences that fit the cap are
/// kept; with no sentence end inside the cap, a word cut ending in "…".
#[test]
fn normalize_narrative_caps_long_answers_at_a_sentence_end() {
    let exact = "x".repeat(NARRATIVE_CHAR_CAP);
    assert_eq!(normalize_narrative(&exact), Some(exact.clone()));
    let (a, filler) = (
        "You spent the day on the billing migration.",
        "word ".repeat(80),
    );
    // A sentence ending on the cap's last char is kept whole.
    let at_cap = format!("{a} {}.", "x".repeat(NARRATIVE_CHAR_CAP - a.len() - 2));
    // One ending on the char just past the cap is not: the earlier sentence is kept.
    let past_cap = format!(
        "{a} {}. More text.",
        "X".repeat(NARRATIVE_CHAR_CAP - a.len() - 1)
    );
    assert_eq!(past_cap.chars().nth(NARRATIVE_CHAR_CAP), Some('.'));
    // Chars, not bytes: a 201-char, 402-byte sentence still fits.
    let accented = format!("{}.", "é".repeat(200));
    let cases = [
        // "3.5h" and "e.g. the" are not sentence ends; one before a newline is.
        (
            format!("{a} Then 3.5h of review, e.g. the {filler}"),
            a.to_string(),
        ),
        (format!("{a}\nThen {filler}"), a.to_string()),
        (format!("One! Two? Then {filler}"), "One! Two?".to_string()),
        (format!("{at_cap} More text."), at_cap.clone()),
        (past_cap, a.to_string()),
        (format!("{accented} {}", "Word ".repeat(40)), accented),
    ];
    for (input, want) in cases {
        assert_eq!(normalize_narrative(&input), Some(want), "{input}");
    }
    // No sentence end in the cap: a word cut at the narrative cap, not the headline's.
    let run_on = normalize_narrative(&filler);
    assert_eq!(run_on, Some(format!("{}…", "word ".repeat(63).trim_end())));
    let one_word = "x".repeat(NARRATIVE_CHAR_CAP + 1);
    let hard_cut = format!("{}…", "x".repeat(NARRATIVE_CHAR_CAP - 1));
    assert_eq!(normalize_narrative(&one_word), Some(hard_cut));
    // A bare "..." is not a sentence end; the word cut counts chars.
    let dotted = format!("... {}", "Wé ".repeat(120));
    let cut = normalize_narrative(&dotted).expect("narrative");
    assert!(
        cut.chars().count() <= NARRATIVE_CHAR_CAP && cut.ends_with("Wé…"),
        "{cut}"
    );
}

/// The model reads the length rule in three places: the preamble, the user
/// prompt, and the schema's field docs. All three say "at most 2", and none
/// brings back the old 2-4 sentence recap.
#[test]
fn every_prompt_surface_asks_for_the_short_read() {
    let preamble = digest_preamble();
    assert!(preamble.contains("at most 2 short sentences, 40 words in total at most"));
    assert!(preamble.contains("do not recount the days"));
    // The turn is the headline's: the narrative must not take it over.
    assert!(preamble.contains("is the headline's turn, not the narrative's"));
    let prompt = build_digest_prompt("day", 0, DAY_MS, DAY_MS, &[], &[], &HashMap::new());
    let schema = serde_json::to_string(&schemars::schema_for!(DigestNarrative)).unwrap();
    for (name, text) in [
        ("preamble", &preamble),
        ("prompt", &prompt),
        ("schema", &schema),
    ] {
        assert!(text.to_lowercase().contains("at most 2"), "{name}: {text}");
        for stale in ["2 to 4", "2-4", "2–4", "shape it took across the days"] {
            assert!(!text.contains(stale), "{name} still says {stale:?}");
        }
    }
}
