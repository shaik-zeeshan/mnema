//! The picker's row list as pure data (port of `lib/chat/model-picker-rows.ts`).
//! quick = Default + starred (1-9) + recent + "Browse all"; browse = folding
//! provider groups; search = any query, grouped, the match marked, plus
//! "use as a model id" rows. Keyboard rows carry a flat `index`.

use std::collections::HashMap;

use crate::chat::format::model_display_name;

pub type EngineRef = (String, String);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Quick,
    Browse,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pick {
    Sentinel,
    Pool(EngineRef),
    Exact(EngineRef),
}

/// `main` split around the search match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Marked {
    pub before: String,
    pub matched: String,
    pub after: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderIcon {
    None,
    Star,
    Clock,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    Header {
        provider: Option<String>,
        label: String,
        icon: HeaderIcon,
        hint: Option<String>,
    },
    Group {
        provider: String,
        label: String,
        count: Option<usize>,
        open: bool,
        index: usize,
    },
    Browse {
        label: String,
        detail: Option<String>,
        index: usize,
    },
    Back {
        index: usize,
    },
    Loading {
        label: String,
    },
    Failure {
        text: String,
    },
    Note {
        text: String,
    },
    Option(OptionRow),
}

#[derive(Clone, Debug, PartialEq)]
pub struct OptionRow {
    pub pick: Pick,
    pub glyph: String,
    pub main: Marked,
    pub sub: Option<String>,
    pub title: String,
    pub selected: bool,
    /// Starred state, None when it can't be starred (Default, a typed id).
    pub starred: Option<bool>,
    /// 1-9 on a starred row in the quick view.
    pub num: Option<usize>,
    /// Offer "Make default" (where the parent allows it, never on the current one).
    pub can_make_default: bool,
    pub index: usize,
}

impl Row {
    /// The keyboard index, for rows the arrows reach.
    pub fn index(&self) -> Option<usize> {
        match self {
            Row::Group { index, .. } | Row::Browse { index, .. } | Row::Back { index } => {
                Some(*index)
            }
            Row::Option(o) => Some(o.index),
            _ => None,
        }
    }
}

pub struct Sentinel {
    pub label: String,
    pub detail: Option<String>,
    pub title: String,
    pub selected: bool,
    pub glyph: String,
}

pub struct Input<'a> {
    pub query: &'a str,
    pub mode: Mode,
    pub group_open: &'a HashMap<String, bool>,
    pub provider_order: &'a [String],
    pub engines: &'a [EngineRef],
    pub context_window: &'a dyn Fn(&str, &str) -> Option<u64>,
    pub provider_label: &'a dyn Fn(&str) -> String,
    pub letter: &'a dyn Fn(&str) -> String,
    pub connected: &'a [String],
    pub sentinel: Option<Sentinel>,
    pub selected_provider: Option<&'a str>,
    pub selected_model: Option<&'a str>,
    pub favorites: &'a [EngineRef],
    pub recents: &'a [EngineRef],
    pub can_star: bool,
    /// `Some(current default)` when rows offer "Make default".
    pub make_default_for: Option<Option<&'a EngineRef>>,
    pub loading: bool,
    pub failures: &'a [(String, String, String)],
}

/// How many recent picks the quick view shows (`QUICK_RECENTS`).
pub const QUICK_RECENTS: usize = 2;

pub fn has_quick_view(favorites: &[EngineRef], recents: &[EngineRef]) -> bool {
    !favorites.is_empty() || !recents.is_empty()
}

fn mark(text: &str, q: &str) -> Marked {
    let at = if q.is_empty() {
        None
    } else {
        text.to_lowercase().find(q)
    };
    match at {
        // Lowercasing can shift byte offsets for non-ASCII; fall back to unmarked.
        Some(i) if text.is_char_boundary(i) && text.is_char_boundary(i + q.len()) => Marked {
            before: text[..i].into(),
            matched: text[i..i + q.len()].into(),
            after: text[i + q.len()..].into(),
        },
        _ => Marked {
            before: text.into(),
            matched: String::new(),
            after: String::new(),
        },
    }
}

pub fn build(input: Input) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut index = 0usize;
    let raw = input.query.trim();
    let q = raw.to_lowercase();
    let is_selected = |provider: &str, model: &str| {
        Some(model) == input.selected_model && input.selected_provider.is_none_or(|p| p == provider)
    };
    let add_engine = |rows: &mut Vec<Row>,
                      index: &mut usize,
                      provider: &str,
                      model: &str,
                      num: Option<usize>,
                      mark_query: bool| {
        let engine = (provider.to_string(), model.to_string());
        rows.push(Row::Option(OptionRow {
            pick: Pick::Pool(engine.clone()),
            glyph: (input.letter)(provider),
            main: mark(&model_display_name(model), if mark_query { &q } else { "" }),
            sub: (input.context_window)(provider, model)
                .map(|c| format!("{}k", (c as f64 / 1000.).round())),
            title: format!("{} · {model}", (input.provider_label)(provider)),
            selected: is_selected(provider, model),
            starred: input.can_star.then(|| input.favorites.contains(&engine)),
            num,
            can_make_default: input
                .make_default_for
                .is_some_and(|current| current != Some(&engine)),
            index: *index,
        }));
        *index += 1;
    };
    let add_sentinel = |rows: &mut Vec<Row>, index: &mut usize| {
        if let Some(s) = &input.sentinel {
            rows.push(Row::Option(OptionRow {
                pick: Pick::Sentinel,
                glyph: s.glyph.clone(),
                main: mark(&s.label, ""),
                sub: None,
                title: s.title.clone(),
                selected: s.selected,
                starred: None,
                num: None,
                can_make_default: false,
                index: *index,
            }));
            *index += 1;
        }
    };
    let models_of = |provider: &str| -> Vec<String> {
        input
            .engines
            .iter()
            .filter(|(p, _)| p == provider)
            .map(|(_, m)| m.clone())
            .collect()
    };

    // ── search ──
    if !q.is_empty() {
        if input
            .sentinel
            .as_ref()
            .is_some_and(|s| s.label.to_lowercase().contains(&q))
        {
            add_sentinel(&mut rows, &mut index);
        }
        for provider in input.provider_order {
            let label = (input.provider_label)(provider);
            let matches: Vec<String> = models_of(provider)
                .into_iter()
                .filter(|m| {
                    m.to_lowercase().contains(&q)
                        || model_display_name(m).to_lowercase().contains(&q)
                        || label.to_lowercase().contains(&q)
                })
                .collect();
            if matches.is_empty() {
                continue;
            }
            rows.push(Row::Header {
                provider: Some(provider.clone()),
                label,
                icon: HeaderIcon::None,
                hint: None,
            });
            for m in matches {
                add_engine(&mut rows, &mut index, provider, &m, None, true);
            }
        }
        let listed = input.engines.iter().any(|(_, m)| m == raw);
        if !rows.iter().any(|r| matches!(r, Row::Option(_))) {
            rows.push(Row::Note {
                text: format!("No models match “{raw}”."),
            });
        }
        if !listed && !input.connected.is_empty() {
            rows.push(Row::Header {
                provider: None,
                label: "Use as a model id".into(),
                icon: HeaderIcon::None,
                hint: None,
            });
            for provider in input.connected {
                rows.push(Row::Option(OptionRow {
                    pick: Pick::Exact((provider.clone(), raw.to_string())),
                    glyph: (input.letter)(provider),
                    main: mark(raw, ""),
                    sub: Some((input.provider_label)(provider)),
                    title: format!("{} · {raw}", (input.provider_label)(provider)),
                    selected: false,
                    starred: None,
                    num: None,
                    can_make_default: false,
                    index,
                }));
                index += 1;
            }
        }
        return rows;
    }

    let quick = has_quick_view(input.favorites, input.recents);

    // ── quick ──
    if input.mode == Mode::Quick && quick {
        add_sentinel(&mut rows, &mut index);
        if !input.favorites.is_empty() {
            rows.push(Row::Header {
                provider: None,
                label: "Favorites".into(),
                icon: HeaderIcon::Star,
                hint: Some(
                    if input.favorites.len() > 1 {
                        "press 1–9"
                    } else {
                        "press 1"
                    }
                    .into(),
                ),
            });
            for (i, (p, m)) in input.favorites.iter().enumerate() {
                add_engine(&mut rows, &mut index, p, m, (i < 9).then_some(i + 1), false);
            }
        }
        let recents: Vec<&EngineRef> = input
            .recents
            .iter()
            .filter(|r| !input.favorites.contains(r))
            .take(QUICK_RECENTS)
            .collect();
        if !recents.is_empty() {
            rows.push(Row::Header {
                provider: None,
                label: "Recent".into(),
                icon: HeaderIcon::Clock,
                hint: None,
            });
            for (p, m) in recents {
                add_engine(&mut rows, &mut index, p, m, None, false);
            }
        }
        let total = input.engines.len();
        rows.push(Row::Browse {
            label: if total > 0 {
                format!("Browse all {total} models")
            } else {
                "Browse all models".into()
            },
            detail: (input.provider_order.len() > 1)
                .then(|| format!("{} providers", input.provider_order.len())),
            index,
        });
        return rows;
    }

    // ── browse ──
    if quick {
        rows.push(Row::Back { index });
        index += 1;
    } else {
        add_sentinel(&mut rows, &mut index);
    }
    let open_by_default = input
        .selected_provider
        .map(str::to_string)
        .or_else(|| input.provider_order.first().cloned());
    for provider in input.provider_order {
        let label = (input.provider_label)(provider);
        let failure = input.failures.iter().find(|(p, ..)| p == provider);
        let models = models_of(provider);
        let waiting = input.loading && models.is_empty() && failure.is_none();
        let open =
            input.group_open.get(provider).copied().unwrap_or(
                open_by_default.as_deref() == Some(provider.as_str()) || failure.is_some(),
            );
        rows.push(Row::Group {
            provider: provider.clone(),
            label: label.clone(),
            count: (!waiting && failure.is_none()).then_some(models.len()),
            open,
            index,
        });
        index += 1;
        if !open {
            continue;
        }
        if let Some((_, flabel, reason)) = failure {
            rows.push(Row::Failure {
                text: format!("{flabel} — {reason}"),
            });
        } else if waiting {
            rows.push(Row::Loading { label });
        } else {
            for m in models {
                add_engine(&mut rows, &mut index, provider, &m, None, false);
            }
        }
    }
    if !input.loading && input.engines.is_empty() && input.failures.is_empty() {
        rows.push(Row::Note {
            text: "No models yet.".into(),
        });
    }
    rows
}
