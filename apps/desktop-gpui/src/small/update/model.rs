//! Pure view logic for the update window, ported from `lib/update-window.ts`.
//! The status is app-core's `AppUpdateStatus` as camelCase JSON (what the
//! Svelte stub serves, and what `CoreEvent::AppUpdateStatusChanged` carries).

use serde_json::Value;

use crate::settings::model::{format_bytes, Json};

/// The window's eleven drawn states (`update.html` `?state=` keys).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Available,
    Checking,
    UpToDate,
    CheckFailed,
    Downloading,
    DownloadFailed,
    Installing,
    InstallFailed,
    Ready,
    Incompatible,
    Expired,
}

pub fn key(status: Option<&Value>) -> Key {
    let Some(s) = status else {
        return Key::Checking;
    };
    let has_update = !s.at(&["update"]).is_null();
    match s.str_at(&["state"]) {
        Some("idle") if has_update => Key::Available,
        Some("available") => Key::Available,
        Some("upToDate") => Key::UpToDate,
        Some("failed") if !has_update => Key::CheckFailed,
        Some("failed") if s.str_at(&["error", "kind"]) == Some("install") => Key::InstallFailed,
        Some("failed") => Key::DownloadFailed,
        Some("downloading") => Key::Downloading,
        Some("installing") => Key::Installing,
        Some("restartRequired") => Key::Ready,
        Some("incompatible") => Key::Incompatible,
        Some("availableOutOfWindow") => Key::Expired,
        _ => Key::Checking,
    }
}

pub fn kicker(k: Key) -> &'static str {
    match k {
        Key::Available => "Update available",
        Key::Downloading | Key::Installing => "Updating",
        Key::DownloadFailed | Key::InstallFailed => "Update failed",
        Key::Ready => "Ready",
        Key::Expired => "Outside your update window",
        _ => "Software update",
    }
}

/// The headline, keyed on `state` (never on whether a version is present).
pub fn heading(status: Option<&Value>) -> String {
    let v = status.and_then(|s| s.str_at(&["update", "version"]));
    let with = |some: &dyn Fn(&str) -> String, none: &str| v.map_or(none.to_string(), some);
    match status.and_then(|s| s.str_at(&["state"])) {
        Some("downloading") => with(&|v| format!("Downloading {v}"), "Downloading update"),
        Some("installing") => with(&|v| format!("Installing {v}"), "Installing update"),
        Some("restartRequired") => with(
            &|v| format!("{v} is ready to run"),
            "Update is ready to run",
        ),
        Some("failed") => with(
            &|v| format!("Couldn\u{2019}t update to {v}"),
            "Couldn\u{2019}t check for updates",
        ),
        Some("upToDate") => "Mnema is up to date".into(),
        Some("availableOutOfWindow") => with(
            &|v| format!("{v} is past your update window"),
            "Update window has lapsed",
        ),
        Some("incompatible") => with(
            &|v| format!("Mnema {v} isn\u{2019}t compatible with this Mac"),
            "No compatible update for this Mac",
        ),
        Some("available") => with(
            &|v| format!("Mnema {v} is available"),
            "An update is available",
        ),
        Some("idle") => with(
            &|v| format!("Mnema {v} is available"),
            "Checking for updates",
        ),
        _ => "Checking for updates".into(),
    }
}

/// Body copy for the states with no release notes.
pub fn message(k: Key) -> Option<&'static str> {
    match k {
        Key::Checking => Some("Asking the update feed for a newer build. The version you have keeps working."),
        Key::UpToDate => Some("There\u{2019}s nothing new to install \u{2014} you have the newest build."),
        Key::CheckFailed => Some("The check didn\u{2019}t reach the update feed, so there\u{2019}s nothing to install yet. The version you have keeps working."),
        Key::Incompatible => Some("The newest build has no download for this Mac, so there\u{2019}s nothing to install."),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineTone {
    Plain,
    Ok,
    Warn,
    Danger,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineIcon {
    None,
    Power,
    Check,
    Spin,
    CloudOff,
    Alert,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub tone: LineTone,
    pub icon: LineIcon,
    pub text: String,
}

fn line(tone: LineTone, icon: LineIcon, text: impl Into<String>) -> Line {
    Line {
        tone,
        icon,
        text: text.into(),
    }
}

/// The controls band's status lines. A failed install RESOLVES the invoke, so
/// `status.error` is its only report and must always reach a line.
pub fn lines(status: Option<&Value>) -> Vec<Line> {
    let failure = status
        .and_then(|s| s.str_at(&["error", "message"]))
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .unwrap_or("Update failed.");
    let consequence = || {
        line(
            LineTone::Plain,
            LineIcon::Power,
            "Installing stops recording and relaunches Mnema.",
        )
    };
    match key(status) {
        Key::Available => vec![consequence()],
        Key::Checking => vec![line(LineTone::Plain, LineIcon::Spin, "Checking\u{2026}")],
        Key::UpToDate => vec![line(
            LineTone::Plain,
            LineIcon::Check,
            "Mnema checks again every 2 hours.",
        )],
        Key::CheckFailed => vec![line(
            LineTone::Warn,
            LineIcon::CloudOff,
            format!("{failure} Mnema tries again on its own."),
        )],
        Key::DownloadFailed => vec![
            consequence(),
            line(
                LineTone::Danger,
                LineIcon::Alert,
                format!("{failure} Nothing was installed."),
            ),
        ],
        Key::InstallFailed => vec![
            consequence(),
            line(
                LineTone::Danger,
                LineIcon::Alert,
                format!(
                    "{failure} {} is still running.",
                    status
                        .and_then(|s| s.str_at(&["app", "version"]))
                        .unwrap_or("Your version")
                ),
            ),
        ],
        Key::Ready => vec![line(
            LineTone::Ok,
            LineIcon::Check,
            "Downloaded and verified. Restarting stops recording and relaunches Mnema.",
        )],
        Key::Incompatible => vec![line(
            LineTone::Plain,
            LineIcon::Alert,
            "Your current version keeps working.",
        )],
        Key::Expired => vec![line(
            LineTone::Plain,
            LineIcon::None,
            "Already renewed? Mnema checks your license again \u{2014} no new key needed.",
        )],
        Key::Downloading | Key::Installing => vec![],
    }
}

/// Install stays armed on `failed`: a failed download or swap keeps the pending update.
pub fn can_install(status: Option<&Value>, acting: bool) -> bool {
    let Some(s) = status else { return false };
    !acting
        && !s.at(&["update"]).is_null()
        && matches!(s.str_at(&["state"]), Some("available" | "failed"))
}

pub fn can_restart(status: Option<&Value>, acting: bool) -> bool {
    !acting && status.and_then(|s| s.str_at(&["state"])) == Some("restartRequired")
}

/// The channel shows only off Stable.
pub fn channel_line(status: Option<&Value>) -> Option<&'static str> {
    status
        .filter(|s| s.str_at(&["channel"]).is_some_and(|c| c != "stable"))
        .map(|_| "Preview channel")
}

/// `appUpdateProgressText` (decimal units, like `formatBytes`).
pub fn progress_text(status: Option<&Value>) -> String {
    let Some(p) = status.map(|s| s.at(&["progress"])).filter(|p| !p.is_null()) else {
        return String::new();
    };
    let done = p.num_at(&["downloadedBytes"]).unwrap_or(0.);
    match p.num_at(&["contentLengthBytes"]).filter(|n| *n > 0.) {
        Some(total) => format!("{} of {}", format_bytes(done), format_bytes(total)),
        None => format!("{} downloaded", format_bytes(done)),
    }
}

/// Percent 4..=100, or `None` when the length is unknown (indeterminate bar).
pub fn progress_percent(status: Option<&Value>) -> Option<f32> {
    let p = status?.at(&["progress"]);
    let total = p.num_at(&["contentLengthBytes"]).filter(|n| *n > 0.)?;
    let done = p.num_at(&["downloadedBytes"]).unwrap_or(0.);
    Some(((done / total * 100.) as f32).clamp(4., 100.))
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// `formatUpdateDate`: "Nov 3, 2026" from the feed's RFC 3339 date.
pub fn release_date(raw: &str) -> String {
    use time::format_description::well_known::Rfc3339;
    match time::OffsetDateTime::parse(raw, &Rfc3339) {
        Ok(t) => format!(
            "{} {}, {}",
            MONTHS[t.month() as usize - 1],
            t.day(),
            t.year()
        ),
        Err(_) => raw.to_string(),
    }
}

/// "12:04 PM" today, "just now" inside a minute, else "Nov 3, 9:12 AM".
pub fn checked_clock(at_ms: Option<i64>, now: time::OffsetDateTime) -> Option<String> {
    let at_ms = at_ms.filter(|m| *m > 0)?;
    let now_ms = (now.unix_timestamp_nanos() / 1_000_000) as i64;
    if now_ms - at_ms < 60_000 {
        return Some("just now".into());
    }
    let t = time::OffsetDateTime::from_unix_timestamp_nanos(at_ms as i128 * 1_000_000)
        .ok()?
        .to_offset(now.offset());
    let h = match t.hour() % 12 {
        0 => 12,
        h => h,
    };
    let clock = format!(
        "{h}:{:02} {}",
        t.minute(),
        if t.hour() < 12 { "AM" } else { "PM" }
    );
    Some(if t.date() == now.date() {
        clock
    } else {
        format!("{} {}, {clock}", MONTHS[t.month() as usize - 1], t.day())
    })
}

/// One block of GitHub's generated release notes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoteBlock {
    Heading(String),
    /// A bullet run (`- ` / `* `), one entry per item.
    List(Vec<String>),
    /// A meta paragraph (`**Full Changelog**: url`): words, then an optional link.
    Para(String, Option<String>),
}

/// The release notes as GitHub writes them (headings, bullets, a closing
/// paragraph), so the window can set `.up-notes` type exactly. `None` for
/// anything richer (code, tables, quotes, images): that falls back to markdown.
pub fn note_blocks(md: &str) -> Option<Vec<NoteBlock>> {
    let mut out: Vec<NoteBlock> = Vec::new();
    for line in md.lines().map(str::trim) {
        if line.is_empty() {
            continue;
        }
        if line.starts_with("```")
            || line.starts_with('|')
            || line.starts_with('>')
            || line.contains("![")
        {
            return None;
        }
        if let Some(h) = line.strip_prefix('#') {
            out.push(NoteBlock::Heading(inline(h.trim_start_matches('#').trim())));
        } else if let Some(item) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            match out.last_mut() {
                Some(NoteBlock::List(items)) => items.push(inline(item)),
                _ => out.push(NoteBlock::List(vec![inline(item)])),
            }
        } else {
            let text = inline(line);
            out.push(match text.find("https://") {
                Some(i) => NoteBlock::Para(text[..i].to_string(), Some(text[i..].to_string())),
                None => NoteBlock::Para(text, None),
            });
        }
    }
    Some(out)
}

/// Plain text of one line: `**bold**` and `` `code` `` marks dropped, `[t](u)` → `t`.
fn inline(s: &str) -> String {
    let s = s.replace("**", "").replace('`', "");
    let mut out = String::new();
    let mut rest = s.as_str();
    while let Some(i) = rest.find('[') {
        let Some(j) = rest[i..].find("](").map(|j| i + j) else {
            break;
        };
        let Some(k) = rest[j..].find(')').map(|k| j + k) else {
            break;
        };
        out.push_str(&rest[..i]);
        out.push_str(&rest[i + 1..j]);
        rest = &rest[k + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn view_keys_split_failed_three_ways() {
        let s = |v: Value| key(Some(&v));
        assert_eq!(key(None), Key::Checking);
        assert_eq!(s(json!({"state": "idle", "update": null})), Key::Checking);
        assert_eq!(
            s(json!({"state": "idle", "update": {"version": "0.2"}})),
            Key::Available
        );
        assert_eq!(
            s(json!({"state": "failed", "update": null})),
            Key::CheckFailed
        );
        assert_eq!(
            s(json!({"state": "failed", "update": {}, "error": {"kind": "install"}})),
            Key::InstallFailed
        );
        assert_eq!(
            s(json!({"state": "failed", "update": {}, "error": {"kind": "network"}})),
            Key::DownloadFailed
        );
        assert_eq!(s(json!({"state": "restartRequired"})), Key::Ready);
    }

    #[test]
    fn heading_lines_and_arming() {
        let st = json!({"state": "failed", "update": {"version": "0.2.0"},
            "error": {"kind": "install", "message": "Update could not be installed."},
            "app": {"version": "0.1.9"}});
        assert_eq!(heading(Some(&st)), "Couldn\u{2019}t update to 0.2.0");
        let l = lines(Some(&st));
        assert_eq!(
            l[1].text,
            "Update could not be installed. 0.1.9 is still running."
        );
        assert!(can_install(Some(&st), false));
        assert!(!can_install(Some(&st), true));
        assert_eq!(
            heading(Some(&json!({"state": "failed", "update": null}))),
            "Couldn\u{2019}t check for updates"
        );
        let dl = json!({"state": "downloading", "progress": {"downloadedBytes": 21_000_000, "contentLengthBytes": 42_000_000}});
        assert_eq!(progress_text(Some(&dl)), "21.0 MB of 42.0 MB");
        assert_eq!(progress_percent(Some(&dl)), Some(50.));
        assert_eq!(release_date("2026-11-03T10:00:00Z"), "Nov 3, 2026");
    }

    #[test]
    fn github_notes_parse_into_blocks() {
        let md = "## What's Changed\n- One by [@a](https://x)\n* `Two`\n\n**Full Changelog**: https://github.com/o/r/compare/a...b";
        assert_eq!(
            note_blocks(md),
            Some(vec![
                NoteBlock::Heading("What's Changed".into()),
                NoteBlock::List(vec!["One by @a".into(), "Two".into()]),
                NoteBlock::Para(
                    "Full Changelog: ".into(),
                    Some("https://github.com/o/r/compare/a...b".into())
                ),
            ])
        );
        assert_eq!(note_blocks("```\ncode\n```"), None);
    }
}
