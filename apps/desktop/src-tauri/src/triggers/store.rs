//! Trigger definition CRUD over `triggers.json` (issue #182).
//!
//! The management UI's command surface: list/create/update/delete over the
//! same file the evaluator hot-reloads every tick, so a saved change is live
//! within one tick with no event plumbing. Every write is atomic (temp file +
//! rename in the same dir), and every read-modify-write holds one process-wide
//! lock so two concurrent commands can't lose each other's write.
//!
//! - Ids are slugs of the name (`"Meeting Recap"` → `"meeting-recap"`),
//!   suffixed `-2`, `-3`… on collision — matching the hand-authored id style
//!   the pre-#182 files used. Ids are stable across renames: update keeps the
//!   id, only create generates one.
//! - CRUD reads are STRICT: a malformed `triggers.json` is an error here (the
//!   evaluator's lenient read stays as-is) so a write can never silently
//!   clobber a hand-edited file that failed to parse.
//! - `create_trigger` enforces the creation-time Provider Gate server-side —
//!   the wizard shows the gate, the backend guarantees it.
//! - `delete_trigger` cascades to the firing ledger
//!   (`trigger_firings::delete_firings`) and the evaluator cursor
//!   (`trigger_state::clear_last_fired`) — the delete-by-id half of the no-FK
//!   file/DB contract.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use tauri::Manager;

use crate::app_infra::AppInfraState;

use super::{schedule, ScheduleCadence, TriggerCondition, TriggerDefinition, TRIGGERS_FILE_NAME};

/// One writer at a time: every mutation is a whole-file read-modify-write.
/// ponytail: a single process-wide lock; per-file locking has no second file.
static STORE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn triggers_path(app_handle: &tauri::AppHandle) -> Result<PathBuf, String> {
    app_handle
        .path()
        .app_config_dir()
        .map(|dir| dir.join(TRIGGERS_FILE_NAME))
        .map_err(|error| format!("failed to resolve the app config dir: {error}"))
}

/// Strict read for CRUD: missing file = empty (normal first-run state), but a
/// present-yet-malformed file is an ERROR — never "empty", so a follow-up
/// write can't erase a user's hand-edited triggers.
fn read_strict(path: &Path) -> Result<Vec<TriggerDefinition>, String> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("failed to read {path:?}: {error}")),
    };
    serde_json::from_str(&contents).map_err(|error| {
        format!("{path:?} is not a valid trigger definition array ({error}) — fix or remove it, then retry")
    })
}

/// Atomic replace: write a temp file beside the target, then rename over it.
fn write_atomic(path: &Path, triggers: &[TriggerDefinition]) -> Result<(), String> {
    let json = serde_json::to_string_pretty(triggers)
        .map_err(|error| format!("failed to serialize triggers: {error}"))?;
    let dir = path
        .parent()
        .ok_or_else(|| format!("{path:?} has no parent directory"))?;
    std::fs::create_dir_all(dir)
        .map_err(|error| format!("failed to create {dir:?}: {error}"))?;
    let tmp = dir.join(format!("{TRIGGERS_FILE_NAME}.tmp"));
    std::fs::write(&tmp, json).map_err(|error| format!("failed to write {tmp:?}: {error}"))?;
    std::fs::rename(&tmp, path)
        .map_err(|error| format!("failed to replace {path:?}: {error}"))
}

/// Slug of a display name: lowercase alphanumeric runs joined by `-`.
fn slugify(name: &str) -> String {
    let mut slug = String::new();
    let mut pending_dash = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            if pending_dash && !slug.is_empty() {
                slug.push('-');
            }
            pending_dash = false;
            slug.push(ch.to_ascii_lowercase());
        } else {
            pending_dash = true;
        }
    }
    slug
}

/// Generate a stable id from the name's slug, `-2`/`-3`… on collision.
fn generate_trigger_id(name: &str, existing: &[TriggerDefinition]) -> String {
    let base = {
        let slug = slugify(name);
        if slug.is_empty() {
            "trigger".to_string()
        } else {
            slug
        }
    };
    let taken = |id: &str| existing.iter().any(|trigger| trigger.id == id);
    if !taken(&base) {
        return base;
    }
    let mut n = 2u32;
    loop {
        let candidate = format!("{base}-{n}");
        if !taken(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// Shared create/update validation. The wizard enforces the same rules; this
/// is the trust boundary for hand-crafted invokes and imported JSON.
fn validate(name: &str, condition: &TriggerCondition, prompt: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("a trigger needs a name".to_string());
    }
    if prompt.trim().is_empty() {
        return Err("a trigger needs a prompt".to_string());
    }
    match condition {
        TriggerCondition::Schedule {
            cadence,
            time,
            weekday,
        } => {
            if schedule::parse_time_minutes(time).is_none() {
                return Err(format!("{time:?} is not a valid HH:MM time"));
            }
            if *cadence == ScheduleCadence::Weekly && weekday.is_none() {
                return Err("a weekly schedule needs a weekday".to_string());
            }
        }
        TriggerCondition::AppOpened {
            bundle_id,
            app_name,
            ..
        } => {
            if bundle_id.trim().is_empty() || app_name.trim().is_empty() {
                return Err("an app-opened trigger needs an app".to_string());
            }
        }
        TriggerCondition::MeetingEnds { .. } => {}
    }
    Ok(())
}

/// What the wizard sends to create: everything but the generated id and the
/// implicit `enabled: true` / `version: 1`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerDraft {
    pub name: String,
    pub condition: TriggerCondition,
    pub prompt: String,
    #[serde(default)]
    pub cooldown_minutes: Option<u32>,
}

// ── Path-level ops (pure file I/O — unit-tested against a temp dir) ─────────

fn create_at(path: &Path, draft: TriggerDraft) -> Result<TriggerDefinition, String> {
    validate(&draft.name, &draft.condition, &draft.prompt)?;
    let mut all = read_strict(path)?;
    let trigger = TriggerDefinition {
        id: generate_trigger_id(&draft.name, &all),
        name: draft.name.trim().to_string(),
        condition: draft.condition,
        prompt: draft.prompt,
        enabled: true,
        cooldown_minutes: draft.cooldown_minutes,
        version: 1,
    };
    all.push(trigger.clone());
    write_atomic(path, &all)?;
    Ok(trigger)
}

fn update_at(path: &Path, mut trigger: TriggerDefinition) -> Result<TriggerDefinition, String> {
    validate(&trigger.name, &trigger.condition, &trigger.prompt)?;
    trigger.name = trigger.name.trim().to_string();
    let mut all = read_strict(path)?;
    let slot = all
        .iter_mut()
        .find(|existing| existing.id == trigger.id)
        .ok_or_else(|| {
            format!(
                "no trigger '{}' to update — it may have been deleted",
                trigger.id
            )
        })?;
    *slot = trigger.clone();
    write_atomic(path, &all)?;
    Ok(trigger)
}

/// Remove by id. A missing id is a quiet no-op (the file may have been
/// hand-edited underneath the UI) so the DB cascade still runs.
fn delete_at(path: &Path, trigger_id: &str) -> Result<(), String> {
    let mut all = read_strict(path)?;
    let before = all.len();
    all.retain(|trigger| trigger.id != trigger_id);
    if all.len() != before {
        write_atomic(path, &all)?;
    }
    Ok(())
}

// ── Tauri commands ──────────────────────────────────────────────────────────

/// Full definitions for the management UI (the status view is
/// [`super::list_triggers_status`]). Lenient like the evaluator: a malformed
/// file lists as empty rather than erroring the whole page.
#[tauri::command]
pub async fn list_triggers(app_handle: tauri::AppHandle) -> Result<Vec<TriggerDefinition>, String> {
    Ok(super::load_triggers(&app_handle))
}

/// Create a trigger. Enforces the creation-time Provider Gate
/// (docs/triggers/CONTEXT.md): creation is refused while the Reasoning Engine
/// is unconfigured — the same gate the evaluator and wizard use.
#[tauri::command]
pub async fn create_trigger(
    app_handle: tauri::AppHandle,
    draft: TriggerDraft,
) -> Result<TriggerDefinition, String> {
    crate::ask_ai::ensure_ask_ai_access_ready(&app_handle)
        .await
        .map_err(|error| format!("Triggers need a configured AI provider — {error}"))?;
    let path = triggers_path(&app_handle)?;
    let _guard = STORE_LOCK.lock().await;
    create_at(&path, draft)
}

/// Replace a trigger by id (edit, enable/disable). Deliberately NOT provider-
/// gated: disabling or editing an existing trigger must work while the
/// provider is missing — needs-provider is a trigger state, not a lockout.
#[tauri::command]
pub async fn update_trigger(
    app_handle: tauri::AppHandle,
    trigger: TriggerDefinition,
) -> Result<TriggerDefinition, String> {
    let path = triggers_path(&app_handle)?;
    let _guard = STORE_LOCK.lock().await;
    update_at(&path, trigger)
}

/// Delete a trigger and cascade to its DB rows: the firing ledger and the
/// evaluator's last-fired cursor. Past RUNS (conversations) stay — they are
/// ordinary conversations in the chat rail (ADR 0058).
#[tauri::command]
pub async fn delete_trigger(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppInfraState>,
    trigger_id: String,
) -> Result<(), String> {
    let path = triggers_path(&app_handle)?;
    {
        let _guard = STORE_LOCK.lock().await;
        delete_at(&path, &trigger_id)?;
    }
    let infra = std::sync::Arc::clone(&*state);
    infra
        .trigger_firings()
        .delete_firings(&trigger_id)
        .await
        .map_err(|error| format!("deleted the trigger but not its firing ledger: {error}"))?;
    infra
        .trigger_state()
        .clear_last_fired(&trigger_id)
        .await
        .map_err(|error| format!("deleted the trigger but not its evaluator cursor: {error}"))?;
    Ok(())
}

/// The Provider Gate probe for the wizard/list when no triggers exist yet —
/// the SAME gate `create_trigger` and the evaluator enforce.
#[tauri::command]
pub async fn triggers_provider_ready(app_handle: tauri::AppHandle) -> Result<bool, String> {
    Ok(crate::ask_ai::ensure_ask_ai_access_ready(&app_handle)
        .await
        .is_ok())
}

/// The global Meeting release grace in minutes (Settings knob; default 2).
#[tauri::command]
pub async fn get_meeting_release_grace_minutes(
    state: tauri::State<'_, AppInfraState>,
) -> Result<i64, String> {
    Ok(state
        .trigger_state()
        .meeting_release_grace_minutes()
        .await
        .map_err(|error| error.to_string())?
        .unwrap_or(super::meeting_worker::DEFAULT_RELEASE_GRACE_MINUTES))
}

/// Write the global Meeting release grace. The detector worker re-reads it per
/// tick, so the change is live without a restart.
#[tauri::command]
pub async fn set_meeting_release_grace_minutes(
    state: tauri::State<'_, AppInfraState>,
    minutes: i64,
) -> Result<(), String> {
    if !(1..=60).contains(&minutes) {
        return Err("release grace must be between 1 and 60 minutes".to_string());
    }
    state
        .trigger_state()
        .set_meeting_release_grace_minutes(minutes)
        .await
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::super::tests::sample_daily;
    use super::*;
    use crate::triggers::schedule::ScheduleWeekday;

    fn temp_path() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(TRIGGERS_FILE_NAME);
        (dir, path)
    }

    fn draft(name: &str) -> TriggerDraft {
        TriggerDraft {
            name: name.to_string(),
            condition: TriggerCondition::MeetingEnds {
                min_meeting_minutes: None,
            },
            prompt: "Write a recap.".to_string(),
            cooldown_minutes: None,
        }
    }

    #[test]
    fn create_generates_slug_ids_and_round_trips_through_the_file() {
        let (_dir, path) = temp_path();

        let first = create_at(&path, draft("Meeting Recap")).expect("create");
        assert_eq!(first.id, "meeting-recap");
        assert!(first.enabled);
        assert_eq!(first.version, 1);

        // Same name → suffixed id, both survive the file round-trip.
        let second = create_at(&path, draft("Meeting Recap")).expect("create twin");
        assert_eq!(second.id, "meeting-recap-2");
        let third = create_at(&path, draft("Meeting Recap")).expect("create triplet");
        assert_eq!(third.id, "meeting-recap-3");

        let all = read_strict(&path).expect("read back");
        assert_eq!(
            all.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            ["meeting-recap", "meeting-recap-2", "meeting-recap-3"]
        );

        // A symbols-only name still gets an id.
        let odd = create_at(&path, draft("!!!")).expect("create odd");
        assert_eq!(odd.id, "trigger");

        // No temp file left behind by the atomic write.
        assert!(!path.with_file_name(format!("{TRIGGERS_FILE_NAME}.tmp")).exists());
    }

    #[test]
    fn update_replaces_by_id_and_errors_on_a_missing_id() {
        let (_dir, path) = temp_path();
        let created = create_at(&path, draft("Meeting Recap")).expect("create");

        // Edit everything but the id; enabled state rides along (disable).
        let edited = TriggerDefinition {
            name: "Recap v2 ".to_string(),
            prompt: "New prompt.".to_string(),
            enabled: false,
            cooldown_minutes: Some(30),
            ..created.clone()
        };
        let saved = update_at(&path, edited).expect("update");
        assert_eq!(saved.name, "Recap v2"); // trimmed
        let all = read_strict(&path).expect("read back");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, "meeting-recap"); // id stable across rename
        assert!(!all[0].enabled);
        assert_eq!(all[0].cooldown_minutes, Some(30));

        // Missing id → error, file untouched.
        let missing = TriggerDefinition {
            id: "ghost".to_string(),
            ..sample_daily()
        };
        assert!(update_at(&path, missing).is_err());
        assert_eq!(read_strict(&path).expect("read back").len(), 1);
    }

    #[test]
    fn delete_removes_the_row_and_tolerates_a_missing_id() {
        let (_dir, path) = temp_path();
        create_at(&path, draft("A")).expect("create a");
        create_at(&path, draft("B")).expect("create b");

        delete_at(&path, "a").expect("delete");
        let all = read_strict(&path).expect("read back");
        assert_eq!(all.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["b"]);

        // Deleting an id that isn't there (hand-edited file) is a quiet no-op
        // so the DB cascade can still run.
        delete_at(&path, "ghost").expect("no-op delete");
        assert_eq!(read_strict(&path).expect("read back").len(), 1);
    }

    #[test]
    fn a_malformed_file_errors_instead_of_being_clobbered() {
        let (_dir, path) = temp_path();
        std::fs::write(&path, "not json").expect("seed malformed file");

        assert!(create_at(&path, draft("X")).is_err());
        assert!(update_at(&path, sample_daily()).is_err());
        assert!(delete_at(&path, "x").is_err());

        // The malformed file is still exactly as the user left it.
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "not json");
    }

    #[test]
    fn validation_rejects_bad_drafts() {
        let (_dir, path) = temp_path();

        let mut nameless = draft("   ");
        nameless.name = "  ".to_string();
        assert!(create_at(&path, nameless).is_err());

        let mut promptless = draft("X");
        promptless.prompt = " ".to_string();
        assert!(create_at(&path, promptless).is_err());

        let mut bad_time = draft("X");
        bad_time.condition = TriggerCondition::Schedule {
            cadence: ScheduleCadence::Daily,
            time: "25:99".to_string(),
            weekday: None,
        };
        assert!(create_at(&path, bad_time).is_err());

        let mut weekless = draft("X");
        weekless.condition = TriggerCondition::Schedule {
            cadence: ScheduleCadence::Weekly,
            time: "09:00".to_string(),
            weekday: None,
        };
        assert!(create_at(&path, weekless).is_err());

        let mut appless = draft("X");
        appless.condition = TriggerCondition::AppOpened {
            bundle_id: "".to_string(),
            app_name: "Figma".to_string(),
            away_gap_minutes: None,
        };
        assert!(create_at(&path, appless).is_err());

        // A valid weekly draft passes.
        let mut weekly = draft("X");
        weekly.condition = TriggerCondition::Schedule {
            cadence: ScheduleCadence::Weekly,
            time: "09:00".to_string(),
            weekday: Some(ScheduleWeekday::Friday),
        };
        assert!(create_at(&path, weekly).is_ok());
        assert!(read_strict(&path).expect("read").iter().all(|t| t.id == "x"));
    }

    #[test]
    fn draft_deserializes_the_wizard_wire_shape() {
        // The wizard's create payload: camelCase, no id/enabled/version.
        let draft: TriggerDraft = serde_json::from_value(serde_json::json!({
            "name": "Meeting Recap",
            "condition": { "type": "meeting_ends", "minMeetingMinutes": 10 },
            "prompt": "Write a recap.",
            "cooldownMinutes": 15
        }))
        .expect("draft parses");
        assert_eq!(draft.cooldown_minutes, Some(15));
        assert_eq!(
            draft.condition,
            TriggerCondition::MeetingEnds {
                min_meeting_minutes: Some(10)
            }
        );
    }
}
