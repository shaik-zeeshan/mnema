//! `KeyboardStore.shortcutIssues` (Svelte): what is wrong with each draft
//! binding, in visual order. Any issue blocks the shortcuts save.

use std::collections::HashMap;

use serde_json::Value;

use super::shortcuts::{binding, CATEGORIES};

/// `RESERVED_SHORTCUT_BINDINGS`: (applies to category index, binding, label).
/// `None` = every foreground category (all but "Recording & window").
const RESERVED: [(Option<usize>, &str, &str); 7] = [
    (None, "Escape", "close the active surface"),
    (None, "Tab", "move keyboard focus"),
    (None, "Shift+Tab", "move keyboard focus backward"),
    (Some(2), "ArrowLeft", "move to an older frame"),
    (Some(2), "ArrowRight", "move to a newer frame"),
    (Some(2), "Shift+ArrowLeft", "move 10 frames older"),
    (Some(2), "Shift+ArrowRight", "move 10 frames newer"),
];

/// `normalizeShortcutBinding`: modifiers first, then one known key; `None` = invalid.
/// Returns the canonical form and whether a non-Shift modifier is present.
fn normalize(raw: &str) -> Option<(String, bool)> {
    let (mut primary, mut alt, mut shift, mut key) = (false, false, false, None::<String>);
    for part in raw.split('+').map(str::trim) {
        let flag = match part.to_ascii_lowercase().replace('-', "").as_str() {
            "commandorcontrol" | "cmdorctrl" | "primary" | "command" | "cmd" | "meta"
            | "control" | "ctrl" => &mut primary,
            "alt" | "option" | "opt" => &mut alt,
            "shift" => &mut shift,
            _ => {
                if key.is_some() || part.is_empty() {
                    return None;
                }
                key = Some(normalize_key(part)?);
                continue;
            }
        };
        if *flag || key.is_some() {
            return None;
        }
        *flag = true;
    }
    let key = key?;
    let mut parts: Vec<&str> = Vec::new();
    if primary {
        parts.push("CommandOrControl");
    }
    if alt {
        parts.push("Alt");
    }
    if shift {
        parts.push("Shift");
    }
    parts.push(&key);
    Some((parts.join("+"), primary || alt))
}

pub(super) fn normalize_key(key: &str) -> Option<String> {
    let lower = key.to_ascii_lowercase();
    Some(match lower.as_str() {
        "esc" | "escape" => "Escape".into(),
        "space" | "spacebar" => "Space".into(),
        "left" | "arrowleft" => "ArrowLeft".into(),
        "right" | "arrowright" => "ArrowRight".into(),
        "up" | "arrowup" => "ArrowUp".into(),
        "down" | "arrowdown" => "ArrowDown".into(),
        "tab" => "Tab".into(),
        "enter" | "return" => "Enter".into(),
        "backspace" => "Backspace".into(),
        "delete" => "Delete".into(),
        _ if key.chars().count() == 1 => key.to_ascii_uppercase(),
        _ if lower.starts_with('f') && lower[1..].parse::<u8>().is_ok() => key.to_ascii_uppercase(),
        _ => return None,
    })
}

/// `shortcutConflictScope`: 0 global (background), 1 foreground, 2 dashboard, 3 audio drawer.
fn scopes_conflict(a: usize, b: usize) -> bool {
    a == 0 || b == 0 || a == 1 || b == 1 || a == b
}

/// Each action id with an issue → the words shown under its row.
pub fn issues(kb: &Value) -> HashMap<&'static str, String> {
    let mut out: HashMap<&'static str, String> = HashMap::new();
    let mut seen: HashMap<String, Vec<(usize, &'static str, &'static str)>> = HashMap::new();
    for (cat, (_, actions)) in CATEGORIES.iter().enumerate() {
        for action in actions.iter() {
            let raw = binding(kb, action.path);
            let raw = raw.trim();
            if raw.is_empty() {
                continue;
            }
            let Some((normalized, has_modifier)) = normalize(raw) else {
                out.insert(
                    action.id,
                    "Use a valid shortcut such as J, \u{2318}K, or \u{2325}\u{2318}P.".into(),
                );
                continue;
            };
            if cat == 0 && !has_modifier {
                out.insert(
                    action.id,
                    "Background shortcuts must include Command/Control or Alt.".into(),
                );
                continue;
            }
            let key = normalized.to_lowercase();
            if let Some((_, _, label)) = RESERVED.iter().find(|(scope, b, _)| {
                b.to_lowercase() == key && scope.map_or(cat != 0, |s| s == cat)
            }) {
                out.insert(action.id, format!("Reserved to {label}."));
                continue;
            }
            let previous = seen.entry(key).or_default();
            let clashing: Vec<_> = previous
                .iter()
                .filter(|(c, _, _)| scopes_conflict(*c, cat))
                .copied()
                .collect();
            if let Some((_, _, first)) = clashing.first() {
                out.insert(action.id, format!("Conflicts with {first}."));
                for (_, id, _) in &clashing {
                    out.insert(id, format!("Conflicts with {}.", action.label));
                }
            }
            previous.push((cat, action.id, action.label));
        }
    }
    out
}

/// The first action, in visual order, that carries an issue (`firstConflictActionId`).
pub fn first(issues: &HashMap<&'static str, String>) -> Option<&'static str> {
    CATEGORIES
        .iter()
        .flat_map(|(_, a)| a.iter())
        .map(|a| a.id)
        .find(|id| issues.contains_key(id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn kb() -> Value {
        serde_json::to_value(app_core::keyboard_bindings::KeyboardBindingsSettings::defaults())
            .unwrap()
    }

    #[test]
    fn defaults_have_no_issues() {
        assert!(issues(&kb()).is_empty());
    }

    #[test]
    fn a_shared_chord_marks_both_rows_and_reserved_and_bare_keys_say_why() {
        let mut k = kb();
        k["appShortcuts"]["openSettings"] = json!("CommandOrControl+D");
        k["appShortcuts"]["openDebug"] = json!("CommandOrControl+D");
        k["globalShortcuts"]["bindings"]["toggleRecording"] = json!("R");
        k["dashboardShortcuts"]["jumpLatest"] = json!("ArrowLeft");
        let i = issues(&k);
        assert_eq!(i["openSettings"], "Conflicts with Open debug.");
        assert_eq!(i["openDebug"], "Conflicts with Open settings.");
        assert_eq!(
            i["toggleRecording"],
            "Background shortcuts must include Command/Control or Alt."
        );
        assert_eq!(
            i["dashboard.jumpLatest"],
            "Reserved to move to an older frame."
        );
        assert_eq!(first(&i), Some("toggleRecording"));
        // The dashboard and the audio drawer never share a screen: no clash.
        let mut k = kb();
        k["dashboardShortcuts"]["jumpLatest"] = json!("K");
        k["audioDrawerShortcuts"]["playPause"] = json!("K");
        assert!(issues(&k).is_empty());
    }
}
