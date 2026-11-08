use std::cell::RefCell;

use app_core::keyboard_bindings::KeyboardBindingsSettings;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};

use super::*;

/// Records calls; refuses any shortcut listed in `taken`.
#[derive(Default)]
struct FakeOs {
    taken: Vec<HotKey>,
    calls: RefCell<Vec<(&'static str, HotKey)>>,
}

impl Registrar for FakeOs {
    fn register(&self, hotkey: HotKey) -> Result<(), String> {
        self.calls.borrow_mut().push(("register", hotkey));
        if self.taken.contains(&hotkey) {
            Err("taken".into())
        } else {
            Ok(())
        }
    }
    fn unregister(&self, hotkey: HotKey) -> Result<(), String> {
        self.calls.borrow_mut().push(("unregister", hotkey));
        Ok(())
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
