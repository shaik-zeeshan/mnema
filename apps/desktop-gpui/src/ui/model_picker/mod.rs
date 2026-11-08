//! The one model picker (`ModelPickerMenu.svelte`): a trigger over a search box
//! and the model list. Shared by the Chat composer's per-chat pin and Settings'
//! default-model fields (S13), so they never drift apart.
//!
//! Like every `ui::` control it's a `RenderOnce` builder: the parent owns the
//! pool, the selection, the starred / recent lists and what a pick means, and
//! passes them in each frame; open / query / highlight / view live in keyed state.
//!
//! ```ignore
//! ModelPicker::new("chat-model", spec)
//!     .trigger(Trigger::Inline { glyph, label, local })
//!     .on_open(|window, cx| /* (re)list the pool */)
//!     .on_select(|pick, window, cx| /* None = the "Default" row */)
//!     .on_favorite(|engine, window, cx| /* star / unstar */)
//! ```

pub mod rows;
mod view;

use std::collections::HashMap;
use std::rc::Rc;

use capture_types::AiProviderConfig;
use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::{App, Context, ElementId, Entity, SharedString, Window};
use serde::Deserialize;

use crate::chat::format::{provider_is_local, provider_label_by_id};
pub use rows::{EngineRef, Mode, Pick, Row};

/// One listed model (`AiRuntimeModel`'s wire shape).
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PoolModel {
    pub id: String,
    pub provider: String,
    #[serde(default)]
    pub context_window: Option<u64>,
}

/// A provider that failed to list (`AiRuntimeProviderFailure`).
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ProviderFailure {
    pub provider: String,
    pub reason: String,
}

/// What the picker shows. Rebuilt by the parent every frame.
#[derive(Clone, Debug, Default)]
pub struct Spec {
    pub providers: Vec<AiProviderConfig>,
    pub pool: Vec<PoolModel>,
    pub loading: bool,
    pub failures: Vec<ProviderFailure>,
    pub favorites: Vec<EngineRef>,
    pub recents: Vec<EngineRef>,
    /// The "Default" row's label and what it resolves to; None hides it.
    pub sentinel: Option<(String, Option<EngineRef>)>,
    pub sentinel_selected: bool,
    pub selected: Option<(Option<String>, String)>,
    /// The provider whose group sorts first (the default's).
    pub first_provider: Option<String>,
    /// The quiet footer line ("applies to this chat only").
    pub foot: Option<SharedString>,
    pub can_star: bool,
}

/// The trigger's look.
#[derive(Clone, Debug)]
pub enum Trigger {
    /// Chat's ghost button: provider letter, mono label, cloud/local chip.
    Inline {
        glyph: Option<String>,
        label: String,
        local: Option<bool>,
        title: String,
    },
    /// Settings' full-width form control.
    Block { label: String, placeholder: bool },
}

pub(crate) type OnSelect = Rc<dyn Fn(Option<EngineRef>, &mut Window, &mut App)>;
pub(crate) type OnEngine = Rc<dyn Fn(EngineRef, &mut Window, &mut App)>;
pub(crate) type OnOpen = Rc<dyn Fn(&mut Window, &mut App)>;

/// Kept across frames per picker.
pub struct PickerState {
    pub open: bool,
    pub highlight: usize,
    pub mode: Mode,
    pub groups: HashMap<String, bool>,
    pub search: Entity<InputState>,
}

#[derive(IntoElement)]
pub struct ModelPicker {
    pub(crate) id: ElementId,
    pub(crate) spec: Spec,
    pub(crate) trigger: Trigger,
    /// Open upward (the composer docked at the window bottom).
    pub(crate) up: bool,
    pub(crate) force_open: bool,
    pub(crate) on_select: Option<OnSelect>,
    pub(crate) on_favorite: Option<OnEngine>,
    pub(crate) on_open: Option<OnOpen>,
}

impl ModelPicker {
    pub fn new(id: impl Into<ElementId>, spec: Spec) -> Self {
        Self {
            id: id.into(),
            spec,
            trigger: Trigger::Block {
                label: "Choose a model".into(),
                placeholder: true,
            },
            up: false,
            force_open: false,
            on_select: None,
            on_favorite: None,
            on_open: None,
        }
    }
    pub fn trigger(mut self, t: Trigger) -> Self {
        self.trigger = t;
        self
    }
    pub fn up(mut self, on: bool) -> Self {
        self.up = on;
        self
    }
    /// Goldens / the gallery: render open.
    pub fn force_open(mut self) -> Self {
        self.force_open = true;
        self
    }
    pub fn on_select(
        mut self,
        f: impl Fn(Option<EngineRef>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_select = Some(Rc::new(f));
        self
    }
    pub fn on_favorite(mut self, f: impl Fn(EngineRef, &mut Window, &mut App) + 'static) -> Self {
        self.on_favorite = Some(Rc::new(f));
        self
    }
    pub fn on_open(mut self, f: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_open = Some(Rc::new(f));
        self
    }
}

impl Spec {
    /// Provider letter ("A" for Anthropic).
    pub fn letter(&self, id: &str) -> String {
        provider_label_by_id(&self.providers, id)
            .trim()
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_else(|| "?".into())
    }

    pub fn is_local(&self, id: &str) -> bool {
        provider_is_local(self.providers.iter().find(|p| p.id == id))
    }

    /// The default's provider first, then config order, then pool-only ones.
    pub fn provider_order(&self) -> Vec<String> {
        let mut ids: Vec<String> = Vec::new();
        let mut add = |id: &str| {
            if !ids.iter().any(|i| i == id) {
                ids.push(id.to_string());
            }
        };
        if let Some(f) = &self.first_provider {
            add(f);
        }
        for p in &self.providers {
            add(&p.id);
        }
        for m in &self.pool {
            add(&m.provider);
        }
        ids
    }

    /// The pool as unique `(provider, model)` pairs.
    pub fn engines(&self) -> Vec<EngineRef> {
        let mut out: Vec<EngineRef> = Vec::new();
        for m in &self.pool {
            let e = (m.provider.clone(), m.id.trim().to_string());
            if !e.1.is_empty() && !out.contains(&e) {
                out.push(e);
            }
        }
        out
    }

    pub fn rows(&self, query: &str, mode: Mode, groups: &HashMap<String, bool>) -> Vec<Row> {
        let order = self.provider_order();
        let engines = self.engines();
        let connected: Vec<String> = self.providers.iter().map(|p| p.id.clone()).collect();
        let failures: Vec<(String, String, String)> = self
            .failures
            .iter()
            .map(|f| {
                (
                    f.provider.clone(),
                    provider_label_by_id(&self.providers, &f.provider),
                    f.reason.clone(),
                )
            })
            .collect();
        let ctx = |p: &str, m: &str| {
            self.pool
                .iter()
                .find(|x| x.provider == p && x.id == m)
                .and_then(|x| x.context_window)
        };
        let label = |id: &str| provider_label_by_id(&self.providers, id);
        let letter = |id: &str| self.letter(id);
        let (sp, sm) = match &self.selected {
            Some((p, m)) => (p.as_deref(), Some(m.as_str())),
            None => (None, None),
        };
        rows::build(rows::Input {
            query,
            mode,
            group_open: groups,
            provider_order: &order,
            engines: &engines,
            context_window: &ctx,
            provider_label: &label,
            letter: &letter,
            connected: &connected,
            sentinel: self.sentinel.as_ref().map(|(l, resolves)| rows::Sentinel {
                label: l.clone(),
                detail: resolves.as_ref().map(|(_, m)| m.clone()),
                title: match resolves {
                    Some((_, m)) => format!("{l} · {m}"),
                    None => l.clone(),
                },
                selected: self.sentinel_selected,
                glyph: match (resolves, &self.first_provider) {
                    (Some(_), Some(f)) => self.letter(f),
                    _ => "–".into(),
                },
            }),
            selected_provider: sp,
            selected_model: sm,
            favorites: &self.favorites,
            recents: &self.recents,
            can_star: self.can_star,
            loading: self.loading,
            failures: &failures,
        })
    }
}

/// The picker's keyed state under `id`.
pub fn state(id: &ElementId, window: &mut Window, cx: &mut App) -> Entity<PickerState> {
    window.use_keyed_state(
        ElementId::Name(format!("{id}-picker").into()),
        cx,
        |window, cx: &mut Context<PickerState>| PickerState {
            open: false,
            highlight: 0,
            mode: Mode::Quick,
            groups: HashMap::new(),
            search: cx
                .new(|cx| InputState::new(window, cx).placeholder("Search models or type an id")),
        },
    )
}
