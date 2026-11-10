use std::cell::RefCell;

use app_core::keyboard_bindings::KeyboardBindingsSettings;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};

use super::*;

/// Records calls and what the OS holds for us; refuses any shortcut in `taken`
/// (another app has it) and, like Carbon, one we already hold.
#[derive(Default)]
struct FakeOs {
    taken: Vec<HotKey>,
    calls: RefCell<Vec<(&'static str, HotKey)>>,
    held: RefCell<Vec<HotKey>>,
}

impl Registrar for FakeOs {
    fn register(&self, hotkey: HotKey) -> Result<(), String> {
        self.calls.borrow_mut().push(("register", hotkey));
        if self.taken.contains(&hotkey) || self.held.borrow().contains(&hotkey) {
            return Err("taken".into());
        }
        self.held.borrow_mut().push(hotkey);
        Ok(())
    }
    fn unregister(&self, hotkey: HotKey) -> Result<(), String> {
        self.calls.borrow_mut().push(("unregister", hotkey));
        self.held.borrow_mut().retain(|h| *h != hotkey);
        Ok(())
    }
}

impl FakeOs {
    fn holds(&self, hotkey: HotKey) -> bool {
        self.held.borrow().contains(&hotkey)
    }
}

fn cmd_alt(code: Code) -> HotKey {
    HotKey::new(Some(Modifiers::META | Modifiers::ALT), code)
}

#[test]
fn app_core_defaults_convert_to_cmd_alt_in_0_9() {
    // The strings app-core validated with global-hotkey 0.7 (SUPER) become META in 0.9.
    let defaults = KeyboardBindingsSettings::defaults();
    let b = &defaults.global_shortcuts.bindings;
    assert_eq!(convert(&b.toggle_recording), Some(cmd_alt(Code::KeyR)));
    assert_eq!(
        convert(&b.pause_resume_recording),
        Some(cmd_alt(Code::KeyP))
    );
    assert_eq!(convert(&b.toggle_main_window), Some(cmd_alt(Code::KeyM)));
    assert_eq!(convert(&b.toggle_quick_recall), Some(cmd_alt(Code::Space)));
    assert_eq!(
        convert("Command+Shift+K"),
        Some(HotKey::new(
            Some(Modifiers::META | Modifiers::SHIFT),
            Code::KeyK
        ))
    );
    assert_eq!(convert("  "), None);
    assert_eq!(convert("Alt+NotAKey"), None);
}

#[test]
fn plan_follows_enabled_skips_blanks_and_dedupes() {
    let mut s = KeyboardBindingsSettings::defaults();
    let actions: Vec<Action> = plan(&s).iter().map(|e| e.action).collect();
    assert_eq!(
        actions,
        [
            Action::ToggleRecording,
            Action::PauseResumeRecording,
            Action::ToggleMainWindow,
            Action::ToggleQuickRecall
        ]
    );
    assert_eq!(
        plan(&s)[3].label,
        format!(
            "{} (CommandOrControl+Alt+Space)",
            EDITABLE_ACTIONS
                .iter()
                .find(|a| a.id == "toggleQuickRecall")
                .unwrap()
                .label
        )
    );

    s.global_shortcuts.bindings.pause_resume_recording = String::new();
    s.global_shortcuts.bindings.toggle_main_window = "CommandOrControl+Alt+R".into();
    let actions: Vec<Action> = plan(&s).iter().map(|e| e.action).collect();
    assert_eq!(
        actions,
        [Action::ToggleRecording, Action::ToggleQuickRecall]
    );

    s.global_shortcuts.enabled = false;
    assert!(plan(&s).is_empty());
}

#[test]
fn reregister_unregisters_all_then_registers_new_set_and_reports_failures() {
    let os = FakeOs {
        taken: vec![cmd_alt(Code::KeyP)],
        ..Default::default()
    };
    let defaults = KeyboardBindingsSettings::defaults();
    let (first, failed) = reregister(&os, Vec::new(), &defaults);
    assert_eq!(first.len(), 3, "the taken ⌘⌥P is not held");
    assert_eq!(failed.len(), 1);
    assert!(failed[0].ends_with("(CommandOrControl+Alt+P)"));

    os.calls.borrow_mut().clear();
    let mut changed = defaults.clone();
    changed.global_shortcuts.bindings.toggle_recording = "CommandOrControl+Alt+X".into();
    let (second, failed) = reregister(&os, first, &changed);
    let calls = os.calls.borrow();
    // Every held shortcut goes first, then the whole new set.
    assert!(calls[..3].iter().all(|(op, _)| *op == "unregister"));
    assert!(calls[3..].iter().all(|(op, _)| *op == "register"));
    assert_eq!(calls.len(), 3 + 4);
    assert!(second.iter().any(|e| e.hotkey == cmd_alt(Code::KeyX)));
    assert_eq!(failed.len(), 1);
}

#[test]
fn presses_fire_only_after_onboarding() {
    let os = FakeOs::default();
    let (held, _) = reregister(&os, Vec::new(), &KeyboardBindingsSettings::defaults());
    let id = cmd_alt(Code::KeyM).id();
    assert_eq!(action_for(&held, id, false), None);
    assert_eq!(action_for(&held, id, true), Some(Action::ToggleMainWindow));
    assert_eq!(action_for(&held, cmd_alt(Code::KeyZ).id(), true), None);
}

#[test]
fn a_rebind_releases_the_old_key_and_holds_the_new_one() {
    let os = FakeOs::default();
    let mut s = KeyboardBindingsSettings::defaults();
    let (held, failed) = reregister(&os, Vec::new(), &s);
    assert!(failed.is_empty() && os.holds(cmd_alt(Code::KeyR)));

    s.global_shortcuts.bindings.toggle_recording = "CommandOrControl+Alt+X".into();
    let (held, failed) = reregister(&os, held, &s);
    assert!(failed.is_empty());
    assert!(
        !os.holds(cmd_alt(Code::KeyR)),
        "the old key is released at once"
    );
    assert!(os.holds(cmd_alt(Code::KeyX)));
    assert_eq!(os.held.borrow().len(), 4);
    assert_eq!(action_for(&held, cmd_alt(Code::KeyR).id(), true), None);
    assert_eq!(
        action_for(&held, cmd_alt(Code::KeyX).id(), true),
        Some(Action::ToggleRecording)
    );

    // A clear releases it and holds nothing in its place.
    s.global_shortcuts.bindings.toggle_recording = String::new();
    let (held, _) = reregister(&os, held, &s);
    assert!(!os.holds(cmd_alt(Code::KeyX)));
    assert_eq!(held.len(), 3);
}

#[test]
fn a_swap_in_one_save_holds_both_keys_on_their_new_actions() {
    // Every held key is released before any is taken, so Carbon never sees a double.
    let os = FakeOs::default();
    let mut s = KeyboardBindingsSettings::defaults();
    let (held, _) = reregister(&os, Vec::new(), &s);
    let b = &mut s.global_shortcuts.bindings;
    std::mem::swap(&mut b.toggle_recording, &mut b.pause_resume_recording);
    let (held, failed) = reregister(&os, held, &s);
    assert!(failed.is_empty());
    assert_eq!(
        action_for(&held, cmd_alt(Code::KeyR).id(), true),
        Some(Action::PauseResumeRecording)
    );
    assert_eq!(
        action_for(&held, cmd_alt(Code::KeyP).id(), true),
        Some(Action::ToggleRecording)
    );
}

#[test]
fn the_switch_releases_everything_and_brings_it_all_back() {
    let os = FakeOs::default();
    let mut s = KeyboardBindingsSettings::defaults();
    let (held, _) = reregister(&os, Vec::new(), &s);
    s.global_shortcuts.enabled = false;
    let (held, failed) = reregister(&os, held, &s);
    assert!(held.is_empty() && failed.is_empty());
    assert!(os.held.borrow().is_empty(), "nothing system-wide while off");
    s.global_shortcuts.enabled = true;
    let (held, failed) = reregister(&os, held, &s);
    assert!(failed.is_empty());
    assert_eq!(held.len(), 4);
    assert_eq!(os.held.borrow().len(), 4);
}

#[test]
fn a_key_another_app_holds_is_reported_by_action_and_the_rest_stay() {
    let os = FakeOs {
        taken: vec![cmd_alt(Code::KeyK)],
        ..Default::default()
    };
    let mut s = KeyboardBindingsSettings::defaults();
    let (held, _) = reregister(&os, Vec::new(), &s);
    s.global_shortcuts.bindings.toggle_quick_recall = "CommandOrControl+Alt+K".into();
    let (held, failed) = reregister(&os, held, &s);
    // As Tauri: the old key is released (the saved binding is the new one) and the
    // warning names the action and the key it could not get.
    assert_eq!(failed.len(), 1);
    assert!(
        failed[0].ends_with("(CommandOrControl+Alt+K)"),
        "{}",
        failed[0]
    );
    assert!(!os.holds(cmd_alt(Code::Space)));
    assert_eq!(held.len(), 3, "the other three keep working");
    assert_eq!(action_for(&held, cmd_alt(Code::KeyK).id(), true), None);

    // The next save tries again; once it is free the key is held.
    let os = FakeOs::default();
    let (held, failed) = reregister(&os, held, &s);
    assert!(failed.is_empty());
    assert_eq!(
        action_for(&held, cmd_alt(Code::KeyK).id(), true),
        Some(Action::ToggleQuickRecall)
    );
}
