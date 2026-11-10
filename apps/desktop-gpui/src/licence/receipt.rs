//! The deep-link receipt's policy: a port of `license-deeplink-receipt.ts`
//! (`receiptFaceFor`) and of `LicenseDeepLinkModal.svelte`'s episode rules
//! (when it opens, morphs, stays quiet after a dismissal, or re-opens), plus
//! each face's copy. Pure: the view arms the slow timer and renders.

use capture_types::{Activation, LicenseStatus};
use time::{OffsetDateTime, UtcOffset};

use super::banner::days;

/// After this long without a result the working copy softens (the claim poll
/// can take ~30 s, the renewal refresh ~60 s; never spin mute).
pub const SLOW_AFTER: std::time::Duration = std::time::Duration::from_secs(30);

/// Which `mnema://license/*` route started the episode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    Activate,
    Claim,
    Renewed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Face {
    /// Routed, result not landed yet: spinner, dismissible.
    Working,
    /// The receipt verified on this machine.
    Activated {
        owner: String,
        update_through_ms: i64,
    },
    /// Key installed, device confirmation still in flight.
    Pending {
        owner: String,
        provisional_days_left: u32,
    },
    /// The update window moved; `was_ms` = the pre-link date (if we saw one).
    Renewed {
        update_through_ms: i64,
        was_ms: Option<i64>,
    },
    /// At the 3-device cap; the links come from the server.
    OverCap { reset_url: String, buy_url: String },
    /// `license_deep_link_done { outcome: "failed" }`.
    Failed { message: String },
}

/// `receiptFaceFor`: non-licensed statuses always mean "keep waiting".
pub fn face_for(
    flow: Flow,
    baseline: Option<&LicenseStatus>,
    current: Option<&LicenseStatus>,
) -> Face {
    let Some(LicenseStatus::Licensed {
        update_through_ms,
        email,
        name,
        activation,
        ..
    }) = current
    else {
        return Face::Working;
    };
    if let Activation::RefusedOverCap { reset_url, buy_url } = activation {
        return Face::OverCap {
            reset_url: reset_url.clone(),
            buy_url: buy_url.clone(),
        };
    }
    if flow == Flow::Renewed {
        // An unchanged through-date is a pre-extension emit, not the result.
        let was_ms = match baseline {
            Some(LicenseStatus::Licensed {
                update_through_ms, ..
            }) => Some(*update_through_ms),
            _ => None,
        };
        if was_ms == Some(*update_through_ms) {
            return Face::Working;
        }
        return Face::Renewed {
            update_through_ms: *update_through_ms,
            was_ms,
        };
    }
    let owner = if name.is_empty() { email } else { name }.clone();
    match activation {
        Activation::Activated => Face::Activated {
            owner,
            update_through_ms: *update_through_ms,
        },
        Activation::Pending {
            provisional_days_left,
        } => Face::Pending {
            owner,
            provisional_days_left: *provisional_days_left,
        },
        // Unreachable seconds after a link; the slow copy covers it.
        Activation::Lapsed | Activation::RefusedOverCap { .. } => Face::Working,
    }
}

/// `LicenseDeepLinkDone` (app-core's `license_deep_link_done` payload).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Done {
    Failed(String),
    Closed,
}

impl Done {
    pub fn parse(v: &serde_json::Value) -> Option<Done> {
        match v.get("outcome")?.as_str()? {
            "failed" => Some(Done::Failed(
                v.get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or_default()
                    .to_string(),
            )),
            "closed" => Some(Done::Closed),
            _ => None,
        }
    }
}

/// One deep-link episode's view state.
#[derive(Clone, Debug, Default)]
pub struct Receipt {
    pub open: bool,
    pub face: Option<Face>,
    pub slow: bool,
    pub flow: Option<Flow>,
    baseline: Option<LicenseStatus>,
    /// Stays true after a mid-flight dismissal so the episode keeps listening
    /// silently: success stays quiet, only over-cap re-opens.
    episode: bool,
    /// Bumped per episode; late answers carry the one they started in.
    pub generation: u64,
    pub resetting: bool,
    pub reset_note: Option<String>,
}

impl Receipt {
    /// `startEpisode`: open in the working face over the last known status.
    pub fn start(&mut self, flow: Flow, baseline: Option<LicenseStatus>) {
        *self = Receipt {
            open: true,
            face: Some(Face::Working),
            flow: Some(flow),
            baseline,
            episode: true,
            generation: self.generation + 1,
            ..Default::default()
        };
    }

    /// A live `license_status` emit. True when the slow timer should stop.
    pub fn on_status(&mut self, current: Option<&LicenseStatus>) -> bool {
        let Some(flow) = self.flow.filter(|_| self.episode) else {
            return false;
        };
        let next = face_for(flow, self.baseline.as_ref(), current);
        if next == Face::Working {
            return false;
        }
        if self.open {
            self.face = Some(next);
            return true;
        }
        if matches!(next, Face::OverCap { .. }) {
            self.face = Some(next);
            self.open = true;
        } else {
            self.episode = false; // dismissed mid-flight and resolved fine: quiet
        }
        false
    }

    /// `license_deep_link_done`. An actionable failure beats silence, even
    /// after a dismissal; `closed` ends the episode silently.
    pub fn on_done(&mut self, done: Done) {
        if !self.episode {
            return;
        }
        match done {
            Done::Closed => {
                self.open = false;
                self.episode = false;
            }
            Done::Failed(message) => {
                self.face = Some(Face::Failed { message });
                self.open = true;
            }
        }
    }

    /// Esc, the scrim, ×, Dismiss / Continue / Close.
    pub fn close(&mut self) {
        self.open = false;
        if self.face != Some(Face::Working) {
            self.episode = false;
        }
    }

    pub fn listening(&self) -> bool {
        self.episode
    }
}

/// The text one face shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Copy {
    pub title: &'static str,
    pub badge: Option<&'static str>,
    pub who: Option<String>,
    pub sub: Option<String>,
    /// The renewed face's struck-through old date.
    pub was: Option<String>,
}

pub fn copy(face: &Face, flow: Flow, slow: bool, offset: UtcOffset) -> Copy {
    let date = |ms| long_date(ms, offset);
    let base = |title| Copy {
        title,
        badge: None,
        who: None,
        sub: None,
        was: None,
    };
    match face {
        Face::Working => Copy {
            sub: Some(
                if slow {
                    "Still working \u{2014} you can close this; we'll finish in the background."
                } else {
                    match flow {
                        Flow::Activate => "Verifying your license key\u{2026}",
                        Flow::Claim => {
                            "Waiting for your order to be confirmed \u{2014} usually a few seconds."
                        }
                        Flow::Renewed => "Confirming your renewal \u{2014} usually under a minute.",
                    }
                }
                .into(),
            ),
            ..base("Setting up your license")
        },
        Face::Activated {
            owner,
            update_through_ms,
        } => Copy {
            who: Some(format!("Licensed to {owner}")),
            sub: Some(format!(
                "Updates included through {}",
                date(*update_through_ms)
            )),
            ..base("License activated")
        },
        Face::Pending {
            owner,
            provisional_days_left,
        } => Copy {
            badge: Some("Activating\u{2026}"),
            who: Some(format!("Licensed to {owner}")),
            sub: Some(format!(
                "Confirming this device in the background \u{2014} {} to connect. Mnema is fully unlocked meanwhile.",
                days(*provisional_days_left)
            )),
            ..base("License installed")
        },
        Face::Renewed {
            update_through_ms,
            was_ms,
        } => Copy {
            who: Some(format!(
                "Updates included through {}",
                date(*update_through_ms)
            )),
            sub: was_ms.is_none().then(|| "Thanks for renewing.".into()),
            was: was_ms.map(|w| format!("was {}", date(w))),
            ..base("Update window extended")
        },
        Face::OverCap { .. } => Copy {
            who: Some("This license is already active on its 3 devices.".into()),
            sub: Some("Free up a device to activate this Mac, or buy another license.".into()),
            ..base("Device limit reached")
        },
        Face::Failed { message } => Copy {
            sub: Some(message.clone()),
            ..base("Couldn't finish this license link")
        },
    }
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// `fmtReceiptDate` (en-US long form): "November 13, 2024".
pub fn long_date(ms: i64, offset: UtcOffset) -> String {
    OffsetDateTime::from_unix_timestamp_nanos(ms as i128 * 1_000_000)
        .map(|t| {
            let t = t.to_offset(offset);
            format!(
                "{} {}, {}",
                MONTHS[t.month() as usize - 1],
                t.day(),
                t.year()
            )
        })
        .unwrap_or_default()
}

/// `safeExternalUrl`: only `https:` links from the server leave the app.
pub fn safe_external(url: &str) -> Option<&str> {
    url::Url::parse(url)
        .ok()
        .filter(|u| u.scheme() == "https")
        .map(|_| url)
}

/// The reset outcome's note (`submitReset`).
pub fn reset_note(
    result: &Result<capture_types::ResetDevicesOutcome, String>,
    offset: UtcOffset,
) -> String {
    use capture_types::ResetDevicesOutcome as R;
    match result {
        Ok(R::Reset) => "Devices freed \u{2014} finishing activation on this Mac\u{2026}".into(),
        Ok(R::RateLimited {
            retry_at_ms: Some(at),
        }) => format!(
            "Your devices were already reset recently \u{2014} you can reset again on {}.",
            long_date(*at, offset)
        ),
        Ok(R::RateLimited { retry_at_ms: None }) => {
            "Your devices were already reset recently \u{2014} you can reset again later.".into()
        }
        Err(message) => message.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::banner::fixtures::*;
    use super::*;

    const W: Face = Face::Working;

    #[test]
    fn non_licensed_statuses_keep_the_working_face() {
        for status in [
            None,
            Some(TRIAL_NOT_STARTED),
            Some(trial(7)),
            Some(READ_ONLY),
        ] {
            assert_eq!(face_for(Flow::Activate, None, status.as_ref()), W);
            assert_eq!(face_for(Flow::Claim, None, status.as_ref()), W);
        }
    }

    #[test]
    fn activated_is_the_happy_receipt() {
        assert_eq!(
            face_for(Flow::Claim, Some(&trial(3)), Some(&licensed())),
            Face::Activated {
                owner: "Ada Lovelace".into(),
                update_through_ms: 1_731_536_000_000
            }
        );
    }

    #[test]
    fn owner_falls_back_to_email() {
        let status = licensed_with("", 1, Activation::Activated);
        assert!(matches!(
            face_for(Flow::Activate, None, Some(&status)),
            Face::Activated { owner, .. } if owner == "owner@example.com"
        ));
    }

    #[test]
    fn pending_shows_days_to_connect() {
        assert_eq!(
            face_for(Flow::Claim, None, Some(&licensed_pending(5))),
            Face::Pending {
                owner: "Ada Lovelace".into(),
                provisional_days_left: 5
            }
        );
    }

    #[test]
    fn over_cap_carries_the_server_links() {
        assert_eq!(
            face_for(Flow::Activate, Some(&trial(2)), Some(&licensed_over_cap())),
            Face::OverCap {
                reset_url: "https://license.example/reset".into(),
                buy_url: "https://mnema.day/#pricing".into()
            }
        );
    }

    #[test]
    fn lapsed_stays_working() {
        assert_eq!(face_for(Flow::Activate, None, Some(&licensed_lapsed())), W);
    }

    #[test]
    fn renewed_flow() {
        let before = licensed_with("Ada Lovelace", 1_700_000_000_000, Activation::Activated);
        let after = licensed();
        assert_eq!(face_for(Flow::Renewed, Some(&before), Some(&before)), W);
        assert_eq!(
            face_for(Flow::Renewed, Some(&before), Some(&after)),
            Face::Renewed {
                update_through_ms: 1_731_536_000_000,
                was_ms: Some(1_700_000_000_000)
            }
        );
        assert_eq!(
            face_for(Flow::Renewed, Some(&trial(1)), Some(&after)),
            Face::Renewed {
                update_through_ms: 1_731_536_000_000,
                was_ms: None
            }
        );
        assert!(matches!(
            face_for(Flow::Renewed, Some(&before), Some(&licensed_over_cap())),
            Face::OverCap { .. }
        ));
    }

    #[test]
    fn done_payloads_parse() {
        let failed = serde_json::json!({ "outcome": "failed", "message": "nope" });
        assert_eq!(Done::parse(&failed), Some(Done::Failed("nope".into())));
        assert_eq!(
            Done::parse(&serde_json::json!({ "outcome": "closed" })),
            Some(Done::Closed)
        );
        assert_eq!(Done::parse(&serde_json::json!({ "outcome": "?" })), None);
    }

    #[test]
    fn episode_morphs_in_place_and_ends_on_close() {
        let mut r = Receipt::default();
        r.start(Flow::Activate, Some(trial(3)));
        assert!(r.open && r.face == Some(W));
        // The boot-style trial emit keeps waiting.
        assert!(!r.on_status(Some(&trial(3))));
        assert_eq!(r.face, Some(W));
        assert!(r.on_status(Some(&licensed())));
        assert!(matches!(r.face, Some(Face::Activated { .. })));
        r.close();
        assert!(!r.open && !r.listening());
        // A late answer never lands on the closed receipt.
        r.on_status(Some(&licensed_over_cap()));
        r.on_done(Done::Failed("late".into()));
        assert!(!r.open);
    }

    #[test]
    fn dismissed_mid_flight_stays_quiet_unless_over_cap_or_failed() {
        let mut r = Receipt::default();
        r.start(Flow::Claim, None);
        r.close();
        assert!(!r.open && r.listening(), "still listening silently");
        r.on_status(Some(&licensed()));
        assert!(!r.open && !r.listening(), "success stays quiet");

        r.start(Flow::Claim, None);
        r.close();
        r.on_status(Some(&licensed_over_cap()));
        assert!(r.open && matches!(r.face, Some(Face::OverCap { .. })));

        r.start(Flow::Renewed, None);
        r.close();
        r.on_done(Done::Failed("We couldn't confirm your renewal yet.".into()));
        assert!(r.open && matches!(r.face, Some(Face::Failed { .. })));
        r.close();
        assert!(!r.listening());
    }

    #[test]
    fn closed_done_ends_the_episode_silently() {
        let mut r = Receipt::default();
        r.start(Flow::Activate, None);
        r.on_done(Done::Closed);
        assert!(!r.open && !r.listening());
    }

    #[test]
    fn a_new_episode_resets_and_bumps_the_generation() {
        let mut r = Receipt::default();
        r.start(Flow::Activate, None);
        r.resetting = true;
        r.reset_note = Some("x".into());
        let g = r.generation;
        r.start(Flow::Renewed, None);
        assert_eq!(r.generation, g + 1);
        assert!(!r.resetting && r.reset_note.is_none() && !r.slow);
    }

    #[test]
    fn copy_per_face() {
        let utc = UtcOffset::UTC;
        assert_eq!(
            copy(&W, Flow::Claim, false, utc).sub.as_deref(),
            Some("Waiting for your order to be confirmed \u{2014} usually a few seconds.")
        );
        assert!(copy(&W, Flow::Activate, true, utc)
            .sub
            .unwrap()
            .starts_with("Still working"));
        let a = copy(
            &face_for(Flow::Activate, None, Some(&licensed())),
            Flow::Activate,
            false,
            utc,
        );
        assert_eq!(a.title, "License activated");
        assert_eq!(a.who.as_deref(), Some("Licensed to Ada Lovelace"));
        assert_eq!(
            a.sub.as_deref(),
            Some("Updates included through November 13, 2024")
        );
        let p = copy(
            &face_for(Flow::Claim, None, Some(&licensed_pending(1))),
            Flow::Claim,
            false,
            utc,
        );
        assert_eq!(p.badge, Some("Activating\u{2026}"));
        assert!(p.sub.unwrap().contains("1 day to connect"));
        let r = copy(
            &Face::Renewed {
                update_through_ms: 1_731_536_000_000,
                was_ms: Some(1_700_000_000_000),
            },
            Flow::Renewed,
            false,
            utc,
        );
        assert_eq!(r.was.as_deref(), Some("was November 14, 2023"));
        assert_eq!(r.sub, None);
    }

    #[test]
    fn only_https_links_leave_the_app() {
        assert!(safe_external("https://license.example/reset").is_some());
        assert!(safe_external("http://license.example/reset").is_none());
        assert!(safe_external("file:///etc/passwd").is_none());
        assert!(safe_external("not a url").is_none());
    }

    #[test]
    fn reset_notes() {
        use capture_types::ResetDevicesOutcome as R;
        let utc = UtcOffset::UTC;
        assert!(reset_note(&Ok(R::Reset), utc).starts_with("Devices freed"));
        assert!(reset_note(
            &Ok(R::RateLimited {
                retry_at_ms: Some(1_731_536_000_000)
            }),
            utc
        )
        .ends_with("on November 13, 2024."));
        assert!(reset_note(&Ok(R::RateLimited { retry_at_ms: None }), utc).ends_with("later."));
        assert_eq!(reset_note(&Err("boom".into()), utc), "boom");
    }
}
