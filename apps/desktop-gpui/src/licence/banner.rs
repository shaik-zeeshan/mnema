//! The shell banner's policy: a port of `licensing-banner.ts` (`bannerFor`,
//! `bannerVisible`, `days`) plus `LicenseBanner.svelte`'s if-chain (copy,
//! tone, actions, and low disk as the last resort). The view only renders it.

use capture_types::{Activation, LicenseStatus};

use crate::ui::Tone;

pub fn days(n: u32) -> String {
    format!("{n} {}", if n == 1 { "day" } else { "days" })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrialTone {
    Info,
    Warn,
    Urgent,
}

/// What the licence part of the banner shows, in precedence order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Banner {
    ReadOnly,
    Revoked,
    Lapsed,
    Provisional {
        days_left: u32,
    },
    Trial {
        days_left: u32,
        tone: TrialTone,
        message: String,
    },
}

impl Banner {
    /// `None` for the firm states; the day count for dismissible ones, so a
    /// fresh escalation (3 → 2) re-surfaces a dismissed banner.
    pub fn dismiss_key(&self) -> Option<u32> {
        match self {
            Banner::Provisional { days_left } | Banner::Trial { days_left, .. } => Some(*days_left),
            _ => None,
        }
    }
}

/// `bannerFor`: readOnly → revoked → lapsed activation → provisional nudge
/// (≤3 days) → final-week trial (≤7 days).
pub fn banner_for(status: Option<&LicenseStatus>) -> Option<Banner> {
    match status? {
        LicenseStatus::ReadOnly => Some(Banner::ReadOnly),
        LicenseStatus::Revoked => Some(Banner::Revoked),
        LicenseStatus::Licensed { activation, .. } => match activation {
            Activation::Lapsed => Some(Banner::Lapsed),
            // refusedOverCap is deliberately Settings-only: capture still works.
            Activation::Pending {
                provisional_days_left,
            } if *provisional_days_left <= 3 => Some(Banner::Provisional {
                days_left: *provisional_days_left,
            }),
            _ => None,
        },
        LicenseStatus::Trial { days_left, .. } if *days_left <= 7 => {
            let days_left = *days_left;
            let tone = match days_left {
                0..=1 => TrialTone::Urgent,
                2..=3 => TrialTone::Warn,
                _ => TrialTone::Info,
            };
            let lead = if days_left <= 1 {
                "Your free trial ends today.".to_string()
            } else {
                format!("Free trial ends in {}.", days(days_left))
            };
            let message = format!(
                "{lead} After that, Mnema switches to Read-Only Mode \u{2014} your recorded history stays fully searchable; only new recording pauses until you buy."
            );
            Some(Banner::Trial {
                days_left,
                tone,
                message,
            })
        }
        _ => None,
    }
}

/// `bannerVisible`: shows unless dismissible and dismissed at exactly this count.
pub fn visible(banner: Option<&Banner>, dismissed_at: Option<u32>) -> bool {
    banner.is_some_and(|b| b.dismiss_key().is_none() || b.dismiss_key() != dismissed_at)
}

/// What a banner button does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Opens the purchase checkout.
    BuyLicense,
    /// Settings › License.
    EnterLicense,
    /// Manual Receipt Refresh.
    Recheck,
    /// The trial banner's checkout button.
    BuyMnema,
    /// Settings › Storage.
    OpenStorage,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Action::BuyLicense => "Buy a license",
            Action::EnterLicense => "Enter license",
            Action::Recheck => "Re-check license",
            Action::BuyMnema => "Buy Mnema",
            Action::OpenStorage => "Open Storage",
        }
    }
    /// Stable element id (tests and AX find the button by it).
    pub fn id(self) -> &'static str {
        match self {
            Action::BuyLicense => "licence-banner-buy",
            Action::EnterLicense => "licence-banner-enter",
            Action::Recheck => "licence-banner-recheck",
            Action::BuyMnema => "licence-banner-buy-mnema",
            Action::OpenStorage => "licence-banner-storage",
        }
    }
    /// The first button is the plain one; the rest are ghosts.
    pub fn ghost(self) -> bool {
        self == Action::EnterLicense
    }
}

/// One rendered banner: `LicenseBanner.svelte`'s branches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shown {
    pub tone: Tone,
    /// `role="alert"` for the firm states, `status` otherwise.
    pub alert: bool,
    /// The `<b>` lead.
    pub bold: Option<&'static str>,
    pub text: String,
    pub actions: Vec<Action>,
    /// Which dismissal slot the × writes (`None` = no ×).
    pub dismiss: Option<Dismiss>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dismiss {
    Trial(u32),
    Provisional(u32),
}

/// The per-kind dismissal memory (component state, lost on restart, as Svelte).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Dismissed {
    pub trial: Option<u32>,
    pub provisional: Option<u32>,
}

impl Dismissed {
    pub fn apply(&mut self, d: Dismiss) {
        match d {
            Dismiss::Trial(k) => self.trial = Some(k),
            Dismiss::Provisional(k) => self.provisional = Some(k),
        }
    }
}

/// The one banner the shell shows (licensing first, then the low-disk pause).
pub fn shown(
    status: Option<&LicenseStatus>,
    low_disk: bool,
    dismissed: Dismissed,
) -> Option<Shown> {
    let banner = banner_for(status);
    let key = match banner {
        Some(Banner::Provisional { .. }) => dismissed.provisional,
        _ => dismissed.trial,
    };
    let is_visible = visible(banner.as_ref(), key);
    let firm = |bold, text: &str, actions| Shown {
        tone: Tone::Danger,
        alert: true,
        bold: Some(bold),
        text: text.to_string(),
        actions,
        dismiss: None,
    };
    match banner {
        Some(Banner::ReadOnly) => Some(firm(
            "Your trial has ended.",
            READ_ONLY_TEXT,
            vec![Action::BuyLicense, Action::EnterLicense],
        )),
        Some(Banner::Revoked) => Some(firm(
            "This license has been revoked.",
            READ_ONLY_TEXT,
            vec![Action::BuyLicense, Action::EnterLicense],
        )),
        Some(Banner::Lapsed) => Some(firm(
            "We couldn't confirm your license.",
            "Connect to the internet once to finish activation \u{2014} your recorded history stays fully searchable; new recording is paused until then.",
            vec![Action::Recheck, Action::EnterLicense],
        )),
        Some(Banner::Provisional { days_left }) if is_visible => Some(Shown {
            tone: Tone::Warn,
            alert: false,
            bold: Some("Activation still pending."),
            text: format!(
                "Connect to the internet within {} to keep recording.",
                days(days_left)
            ),
            actions: Vec::new(),
            dismiss: Some(Dismiss::Provisional(days_left)),
        }),
        Some(Banner::Trial {
            days_left,
            tone,
            message,
        }) if is_visible => Some(Shown {
            tone: match tone {
                TrialTone::Urgent => Tone::Danger,
                TrialTone::Warn => Tone::Warn,
                TrialTone::Info => Tone::Info,
            },
            alert: false,
            bold: None,
            text: message,
            actions: vec![Action::BuyMnema],
            dismiss: Some(Dismiss::Trial(days_left)),
        }),
        _ if low_disk => Some(Shown {
            tone: Tone::Warn,
            alert: false,
            bold: Some("Recording paused \u{2014} your disk is almost full."),
            text: "Free up space and it resumes on its own.".into(),
            actions: vec![Action::OpenStorage],
            dismiss: None,
        }),
        _ => None,
    }
}

const READ_ONLY_TEXT: &str =
    "Everything you recorded stays browsable and searchable. Buy a license to resume recording.";

/// `specs/fixtures/license-status.ts`, shared by the licence tests.
#[cfg(test)]
pub(crate) mod fixtures {
    use capture_types::{Activation, LicenseStatus};

    pub const TRIAL_NOT_STARTED: LicenseStatus = LicenseStatus::TrialNotStarted { trial_days: 30 };
    pub const READ_ONLY: LicenseStatus = LicenseStatus::ReadOnly;
    pub const REVOKED: LicenseStatus = LicenseStatus::Revoked;

    pub fn trial(days_left: u32) -> LicenseStatus {
        LicenseStatus::Trial {
            days_left,
            trial_end_ms: 1_700_000_000_000,
        }
    }

    pub fn licensed_with(
        name: &str,
        update_through_ms: i64,
        activation: Activation,
    ) -> LicenseStatus {
        LicenseStatus::Licensed {
            update_through_ms,
            in_window: true,
            email: "owner@example.com".into(),
            name: name.into(),
            activation,
        }
    }

    pub fn licensed() -> LicenseStatus {
        licensed_with("Ada Lovelace", 1_731_536_000_000, Activation::Activated)
    }

    pub fn licensed_pending(provisional_days_left: u32) -> LicenseStatus {
        licensed_with(
            "Ada Lovelace",
            1_731_536_000_000,
            Activation::Pending {
                provisional_days_left,
            },
        )
    }

    pub fn licensed_over_cap() -> LicenseStatus {
        licensed_with(
            "Ada Lovelace",
            1_731_536_000_000,
            Activation::RefusedOverCap {
                reset_url: "https://license.example/reset".into(),
                buy_url: "https://mnema.day/#pricing".into(),
            },
        )
    }

    pub fn licensed_lapsed() -> LicenseStatus {
        licensed_with("Ada Lovelace", 1_731_536_000_000, Activation::Lapsed)
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;

    fn trial_of(n: u32) -> Option<Banner> {
        banner_for(Some(&trial(n)))
    }

    #[test]
    fn trial_thresholds_and_tone_ramp() {
        assert_eq!(trial_of(8), None, "final week only");
        let Some(Banner::Trial {
            days_left,
            tone,
            message,
        }) = trial_of(7)
        else {
            panic!("7 days → trial banner");
        };
        assert_eq!((days_left, tone), (7, TrialTone::Info));
        assert!(message.contains("Free trial ends in 7 days."));
        assert_eq!(trial_of(7).unwrap().dismiss_key(), Some(7));
        let Some(Banner::Trial { tone, message, .. }) = trial_of(3) else {
            panic!()
        };
        assert_eq!(tone, TrialTone::Warn);
        assert!(message.contains("Free trial ends in 3 days."));
        let Some(Banner::Trial { tone, message, .. }) = trial_of(1) else {
            panic!()
        };
        assert_eq!(tone, TrialTone::Urgent);
        assert!(message.contains("Your free trial ends today."));
    }

    #[test]
    fn healthy_states_have_no_banner() {
        assert_eq!(banner_for(None), None);
        assert_eq!(banner_for(Some(&TRIAL_NOT_STARTED)), None);
        assert_eq!(banner_for(Some(&licensed())), None);
        // Over-cap is deliberately Settings-only.
        assert_eq!(banner_for(Some(&licensed_over_cap())), None);
    }

    #[test]
    fn firm_states_are_never_dismissible() {
        assert_eq!(banner_for(Some(&READ_ONLY)), Some(Banner::ReadOnly));
        assert_eq!(banner_for(Some(&REVOKED)), Some(Banner::Revoked));
        assert_eq!(banner_for(Some(&licensed_lapsed())), Some(Banner::Lapsed));
        for b in [Banner::ReadOnly, Banner::Revoked, Banner::Lapsed] {
            assert_eq!(b.dismiss_key(), None);
        }
        assert!(visible(banner_for(Some(&READ_ONLY)).as_ref(), Some(3)));
        assert!(visible(banner_for(Some(&REVOKED)).as_ref(), None));
        assert!(visible(
            banner_for(Some(&licensed_lapsed())).as_ref(),
            Some(1)
        ));
    }

    #[test]
    fn provisional_nudge_only_in_the_last_three_days() {
        assert_eq!(
            banner_for(Some(&licensed_pending(3))),
            Some(Banner::Provisional { days_left: 3 })
        );
        assert_eq!(
            banner_for(Some(&licensed_pending(1)))
                .unwrap()
                .dismiss_key(),
            Some(1)
        );
        assert_eq!(banner_for(Some(&licensed_pending(4))), None);
        assert_eq!(banner_for(Some(&licensed_pending(7))), None);
    }

    #[test]
    fn dismissal_is_keyed_to_the_day_count() {
        assert!(!visible(trial_of(5).as_ref(), Some(5)));
        // Dismissed at 3; the next recompute says 2 → re-surfaces.
        assert!(visible(trial_of(2).as_ref(), Some(3)));
        assert!(visible(
            banner_for(Some(&licensed_pending(2))).as_ref(),
            Some(3)
        ));
        assert!(!visible(None, None));
    }

    #[test]
    fn days_is_singular_and_plural() {
        assert_eq!(days(1), "1 day");
        assert_eq!(days(2), "2 days");
    }

    #[test]
    fn shown_copy_tone_and_actions_match_the_component() {
        let none = Dismissed::default();
        let ro = shown(Some(&READ_ONLY), false, none).unwrap();
        assert_eq!(
            (ro.tone, ro.alert, ro.bold, ro.dismiss),
            (Tone::Danger, true, Some("Your trial has ended."), None)
        );
        assert_eq!(ro.text, READ_ONLY_TEXT);
        assert_eq!(ro.actions, vec![Action::BuyLicense, Action::EnterLicense]);
        let rv = shown(Some(&REVOKED), true, none).unwrap();
        assert_eq!(
            rv.bold,
            Some("This license has been revoked."),
            "licence outranks low disk"
        );
        let lapsed = shown(Some(&licensed_lapsed()), false, none).unwrap();
        assert_eq!(lapsed.actions, vec![Action::Recheck, Action::EnterLicense]);
        assert_eq!(lapsed.bold, Some("We couldn't confirm your license."));
        let prov = shown(Some(&licensed_pending(2)), false, none).unwrap();
        assert_eq!((prov.tone, prov.alert), (Tone::Warn, false));
        assert_eq!(
            prov.text,
            "Connect to the internet within 2 days to keep recording."
        );
        assert_eq!(prov.dismiss, Some(Dismiss::Provisional(2)));
        assert!(prov.actions.is_empty());
        let urgent = shown(Some(&trial(1)), false, none).unwrap();
        assert_eq!((urgent.tone, urgent.bold), (Tone::Danger, None));
        assert_eq!(urgent.actions, vec![Action::BuyMnema]);
        assert_eq!(urgent.dismiss, Some(Dismiss::Trial(1)));
        assert_eq!(
            shown(Some(&trial(3)), false, none).unwrap().tone,
            Tone::Warn
        );
        assert_eq!(
            shown(Some(&trial(6)), false, none).unwrap().tone,
            Tone::Info
        );
    }

    #[test]
    fn a_dismissed_banner_falls_through_to_low_disk() {
        let mut d = Dismissed::default();
        d.apply(Dismiss::Trial(5));
        assert_eq!(shown(Some(&trial(5)), false, d), None);
        let low = shown(Some(&trial(5)), true, d).unwrap();
        assert_eq!(low.actions, vec![Action::OpenStorage]);
        assert_eq!(low.dismiss, None, "low disk has no ×");
        // The provisional slot is separate from the trial one.
        assert!(shown(Some(&licensed_pending(3)), false, d).is_some());
        d.apply(Dismiss::Provisional(3));
        assert_eq!(shown(Some(&licensed_pending(3)), false, d), None);
        assert!(shown(Some(&licensed_pending(2)), false, d).is_some());
        assert_eq!(shown(Some(&licensed()), false, d), None);
    }
}
