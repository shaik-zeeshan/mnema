//! `SpeakerRepairPanel`: identity repair as a right-edge slide-over (never a
//! modal: you keep reading the lines you're attributing). One search box filters
//! the saved people and offers the typed name as a new one; one primary commits
//! the pick. Every write here has no backend inverse, so there's no undo.

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{
    div, px, Animation, AnimationExt as _, AnyElement, Context, Role, SharedString, Window,
};

use super::data::Write;
use super::header::mark;
use super::reader::REPAIR_W;
use super::view_model::{self as vm};
use super::{AudioDrawer, Repair};
use crate::theme::{self, motion, radius, space, text, Palette, FONT_MONO};
use crate::ui::chip::{chip, ChipKind};
use crate::ui::choice::{Choice, ChoiceOption};
use crate::ui::input::{field_warning, TextInput};
use crate::ui::segmented::Segmented;
use crate::ui::{with_tip, Button};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    Person(i64),
    New,
}

impl AudioDrawer {
    /// The gutter's repair door (fallback paragraphs have no cluster to repair).
    pub fn open_repair(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(g) = self.groups.get(index).filter(|g| g.cluster_id >= 0) else {
            return;
        };
        self.repair = Some(Repair {
            index,
            cluster: g.cluster_id,
            line_scope: false,
            pick: None,
            move_to: None,
            close_on_success: false,
            last: None,
        });
        self.name.update(cx, |i, cx| {
            i.set_value("", window, cx);
            i.focus(window, cx);
        });
        cx.notify();
    }

    pub fn reattach(&mut self) {
        self.followed = None;
    }
}

/// The people the field offers, whether the typed name can be created, and the pick.
pub fn picks(d: &AudioDrawer, query: &str) -> (Vec<(i64, String, String)>, bool, Option<Pick>) {
    let (Some(r), Some(g)) = (
        &d.repair,
        d.repair.as_ref().and_then(|r| d.groups.get(r.index)),
    ) else {
        return (Vec::new(), false, None);
    };
    let q = query.trim();
    let lower = q.to_lowercase();
    let people: Vec<(i64, String, String)> = d
        .data
        .profiles
        .iter()
        .filter(|p| {
            Some(p.id) != g.person_id
                && (q.is_empty() || p.display_name.to_lowercase().contains(&lower))
        })
        .map(|p| {
            let samples = vm::samples_label(p.embedding_count);
            let tag = match (p.is_account_owner, samples.is_empty()) {
                (true, true) => "you".to_string(),
                (true, false) => format!("you · {samples}"),
                _ => samples,
            };
            (p.id, p.display_name.clone(), tag)
        })
        .collect();
    let can_create = !q.is_empty()
        && vm::name_warning(q).is_none()
        && !d
            .data
            .profiles
            .iter()
            .any(|p| p.display_name.to_lowercase() == lower);
    let pick = match r.pick {
        Some(Pick::New) => can_create.then_some(Pick::New),
        Some(Pick::Person(id)) if people.iter().any(|p| p.0 == id) => Some(Pick::Person(id)),
        _ if can_create && people.is_empty() => Some(Pick::New),
        _ => None,
    };
    (people, can_create, pick)
}

fn card(p: &Palette) -> gpui_kit::Div {
    div()
        .flex()
        .flex_col()
        .gap(space::S2)
        .p(space::S3)
        .rounded(radius::MD)
        .bg(p.surface)
        .border_1()
        .border_color(p.border)
}

fn card_title(s: String, p: &Palette) -> gpui_kit::Div {
    div()
        .flex()
        .items_center()
        .gap(space::S2)
        .text_size(text::MD)
        .font_weight(theme::W_SEMIBOLD)
        .text_color(p.text_strong)
        .child(s)
}

fn hint(s: String, p: &Palette) -> gpui_kit::Div {
    div()
        .text_size(text::BASE)
        .line_height(px(12. * 1.45))
        .text_color(p.text_subtle)
        .child(s)
}

fn first(name: &str) -> String {
    name.split(' ').next().unwrap_or(name).to_string()
}

pub fn render(
    d: &AudioDrawer,
    p: &Palette,
    _window: &mut Window,
    cx: &mut Context<AudioDrawer>,
) -> Option<AnyElement> {
    let r = d.repair.as_ref()?;
    let g = d.groups.get(r.index)?.clone();
    let cluster = g.cluster_id;
    let busy = d.busy_cluster.is_some();
    let profiles = &d.data.profiles;
    let name = vm::persisted_name(&g, profiles);
    let linked = profiles.iter().find(|p| Some(p.id) == g.person_id);
    let auto = d
        .data
        .clusters
        .iter()
        .any(|c| c.id == cluster && c.person_link_auto)
        && g.person_id.is_some();
    let suggested = (g.person_id.is_none())
        .then(|| {
            profiles
                .iter()
                .find(|p| Some(p.id) == g.suggested_person_id)
        })
        .flatten();
    let query = d.name.read(cx).value().to_string();
    let warning = (!query.trim().is_empty())
        .then(|| vm::name_warning(&query))
        .flatten();
    let (people, can_create, pick) = picks(d, &query);
    let turn_count = d
        .data
        .turns
        .iter()
        .filter(|t| t.cluster_id == cluster)
        .count();
    let unnamed_left = {
        let mut ids: Vec<i64> = d
            .groups
            .iter()
            .filter(|g| g.cluster_id >= 0 && g.person_id.is_none())
            .map(|g| g.cluster_id)
            .collect();
        ids.sort();
        ids.dedup();
        ids.len().saturating_sub(usize::from(g.person_id.is_none()))
    };
    let not_this = move |d: &mut AudioDrawer, cx: &mut Context<AudioDrawer>| {
        let w = if g.person_id.is_some() {
            Write::Unlink { cluster }
        } else {
            Write::Reject { cluster }
        };
        d.write(cluster, w, false, cx);
    };

    let mut body: Vec<AnyElement> = Vec::new();
    if let Some(s) = suggested {
        let (sid, sname) = (s.id, s.display_name.clone());
        body.push(
            card(p)
                .child(card_title(format!("Sounds like {sname}"), p).children(
                    g.recognition_confidence.clone().map(|c| chip(c, ChipKind::Neutral, p)),
                ))
                .child(hint(
                    format!(
                        "Only a suggestion. Confirm links this voice and saves a sample; “Not {f}” stops suggesting {f} for this voice only.",
                        f = first(&sname)
                    ),
                    p,
                ))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(6.))
                        .child(
                            Button::new("repair-confirm", format!("Confirm {}", first(&sname)))
                                .sm()
                                .icon(IconName::Check.path())
                                .disabled(busy)
                                .on_click(cx.listener(move |d, _, _, cx| {
                                    d.write(cluster, Write::Link { cluster, person: sid }, true, cx)
                                })),
                        )
                        .child(
                            Button::new("repair-not", format!("Not {}", first(&sname)))
                                .sm()
                                .danger()
                                .disabled(busy)
                                .on_click(cx.listener(move |d, _, _, cx| not_this(d, cx))),
                        ),
                )
                .into_any_element(),
        );
    }
    if let (Some(target), false) = (
        vm::merge_target_label(&g, &d.data.clusters, profiles),
        r.line_scope,
    ) {
        let score = d
            .developer
            .then(|| vm::format_score(g.merge_score))
            .flatten();
        let heads = vm::sample_heads(&g, &d.data.turns);
        let merge_into = g.merge_target;
        body.push(
            card(p)
                .child(card_title(format!("Possibly the same voice as {target}"), p))
                .child(hint(
                    format!(
                        "{}Over-segmentation is the common failure — one person split in two. Merging folds every turn of this voice into theirs.",
                        score.map(|s| format!("Centroid similarity {s}. ")).unwrap_or_default()
                    ),
                    p,
                ))
                .child(
                    div()
                        .flex()
                        .gap(px(6.))
                        .child(Button::new("repair-merge", "Merge them").sm().disabled(busy).on_click(
                            cx.listener(move |d, _, _, cx| {
                                if let Some(target) = merge_into {
                                    d.write(cluster, Write::Merge { source: cluster, target }, true, cx)
                                }
                            }),
                        ))
                        .child(
                            Button::new("repair-samples", "Play 8s of each")
                                .sm()
                                .ghost()
                                .icon(IconName::Play.path())
                                .disabled(busy)
                                .on_click(cx.listener(move |d, _, _, cx| {
                                    d.transport.play_samples(&heads);
                                    cx.notify();
                                })),
                        ),
                )
                .into_any_element(),
        );
    }
    body.push(
        div()
            .flex()
            .flex_col()
            .gap(px(7.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_size(text::BASE).font_weight(theme::W_MEDIUM).text_color(p.text_muted).child("Apply to"))
                    .child(
                        Segmented::new("repair-scope", "Repair scope", ["This speaker", "This line only"], usize::from(r.line_scope))
                            .disabled(busy)
                            .on_change({
                                let this = cx.entity().downgrade();
                                move |ix, _, cx| {
                                    this.update(cx, |d, cx| {
                                        if let Some(r) = d.repair.as_mut() {
                                            r.line_scope = ix == 1;
                                        }
                                        cx.notify();
                                    })
                                    .ok();
                                }
                            }),
                    ),
            )
            .child(hint(
                if r.line_scope {
                    "Naming is always speaker-wide, so a single line moves to another speaker instead.".into()
                } else {
                    format!("Every turn this voice holds — {turn_count} in this segment.")
                },
                p,
            ))
            .into_any_element(),
    );
    let this = cx.entity().downgrade();
    if !r.line_scope {
        let mut options: Vec<ChoiceOption> = people
            .iter()
            .map(|(_, n, tag)| {
                let o = ChoiceOption::new(n.clone());
                if tag.is_empty() {
                    o
                } else {
                    o.tag(tag.clone())
                }
            })
            .collect();
        if can_create {
            options.push(ChoiceOption::new(format!("Create “{}”", query.trim())).tag("new person"));
        }
        let selected = match pick {
            Some(Pick::Person(id)) => people.iter().position(|x| x.0 == id).unwrap_or(usize::MAX),
            Some(Pick::New) => people.len(),
            None => usize::MAX,
        };
        let ids: Vec<i64> = people.iter().map(|x| x.0).collect();
        let list = if options.is_empty() {
            if profiles.is_empty() {
                hint(
                    "No saved people yet. Type a name to save this voice as the first one.".into(),
                    p,
                )
                .into_any_element()
            } else {
                hint(format!("No saved person matches “{}”.", query.trim()), p).into_any_element()
            }
        } else {
            Choice::new("repair-people", "Saved people", options, selected)
                .list()
                .on_change(move |ix, _, cx| {
                    let pick = ids.get(ix).map_or(Pick::New, |id| Pick::Person(*id));
                    this.update(cx, |d, cx| {
                        if let Some(r) = d.repair.as_mut() {
                            r.pick = Some(pick);
                        }
                        cx.notify();
                    })
                    .ok();
                })
                .into_any_element()
        };
        body.push(
            div()
                .flex()
                .flex_col()
                .gap(px(7.))
                .child(
                    div()
                        .text_size(text::BASE)
                        .font_weight(theme::W_MEDIUM)
                        .text_color(p.text_muted)
                        .child("Name this voice, or find a saved person"),
                )
                .child(
                    TextInput::new("repair-name", "Name this voice", &d.name)
                        .invalid(warning.is_some())
                        .disabled(busy),
                )
                .children(warning.map(|w| field_warning(w, p)))
                .child(list)
                .into_any_element(),
        );
        if let Some(l) = linked {
            let f = first(&l.display_name);
            body.push(
                div()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(7.))
                    .child(
                        Button::new("repair-unlink", format!("Unlink {f}"))
                            .sm()
                            .danger()
                            .disabled(busy)
                            .on_click(cx.listener(move |d, _, _, cx| not_this(d, cx))),
                    )
                    .child(hint(
                        format!("Unlinks this voice only. {f}’s saved voice and every other recording stay as they are."),
                        p,
                    ))
                    .into_any_element(),
            );
        }
    } else {
        let targets: Vec<_> = d.data.clusters.iter().filter(|c| c.id != cluster).collect();
        let options: Vec<ChoiceOption> = targets
            .iter()
            .map(|c| {
                let n = d.data.turns.iter().filter(|t| t.cluster_id == c.id).count();
                ChoiceOption::new(vm::cluster_option_label(c, profiles)).desc(match n {
                    0 => "elsewhere in this session".to_string(),
                    1 => "1 turn in this segment".to_string(),
                    n => format!("{n} turns in this segment"),
                })
            })
            .collect();
        let ids: Vec<i64> = targets.iter().map(|c| c.id).collect();
        let selected = r
            .move_to
            .and_then(|m| ids.iter().position(|i| *i == m))
            .unwrap_or(usize::MAX);
        body.push(
            div()
                .flex()
                .flex_col()
                .gap(px(7.))
                .child(
                    div()
                        .text_size(text::BASE)
                        .font_weight(theme::W_MEDIUM)
                        .text_color(p.text_muted)
                        .child("Move this line to"),
                )
                .child(
                    Choice::new("repair-move", "Move this line to", options, selected)
                        .list()
                        .on_change(move |ix, _, cx| {
                            let target = ids.get(ix).copied();
                            this.update(cx, |d, cx| {
                                if let Some(r) = d.repair.as_mut() {
                                    r.move_to = target;
                                }
                                cx.notify();
                            })
                            .ok();
                        }),
                )
                .into_any_element(),
        );
    }

    let (label, busy_label, ready) = if r.line_scope {
        ("Move this line".to_string(), "Moving…", r.move_to.is_some())
    } else if let Some(Pick::Person(id)) = pick {
        let n = profiles
            .iter()
            .find(|p| p.id == id)
            .map_or(String::new(), |p| p.display_name.clone());
        (format!("Link to {n}"), "Linking…", true)
    } else {
        (
            "Name & apply".to_string(),
            "Saving…",
            pick == Some(Pick::New),
        )
    };
    let turn_ids = g.turn_ids.clone();
    let commit = {
        let query = query.trim().to_string();
        cx.listener(move |d, _, _, cx| {
            let Some(r) = d.repair.as_ref() else { return };
            let w = if r.line_scope {
                r.move_to.map(|target| Write::MoveTurns {
                    turns: turn_ids.clone(),
                    target,
                })
            } else {
                match pick {
                    Some(Pick::New) => Some(Write::NameAndLink {
                        cluster,
                        name: query.clone(),
                    }),
                    Some(Pick::Person(person)) => Some(Write::Link { cluster, person }),
                    None => None,
                }
            };
            if let Some(w) = w {
                d.write(cluster, w, true, cx);
            }
        })
    };
    let retry = r.last.clone().zip(d.correction_error.clone());
    let close_on = r.close_on_success;
    let panel = div()
        .id("repair")
        .role(Role::Dialog)
        .aria_label(SharedString::from(format!("Speaker repair — {name}")))
        .absolute()
        .top_0()
        .right_0()
        .bottom_0()
        .w(px(REPAIR_W))
        .flex()
        .flex_col()
        .bg(p.surface_raised)
        .border_l_1()
        .border_color(p.border_strong)
        .shadow(crate::ui::popover_shadow(p))
        .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
            cx.stop_propagation()
        })
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(9.))
                .pl(space::S4)
                .pr(space::S2)
                .pt(px(10.))
                .child(mark(d.marks.get(&cluster), vm::is_unnamed(&g), p))
                .child(
                    div()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .text_size(text::MD)
                        .font_weight(theme::W_SEMIBOLD)
                        .text_color(p.text_strong)
                        .child(name.clone()),
                )
                .when(linked.is_some_and(|l| l.is_account_owner), |el| {
                    el.child(chip("you", ChipKind::Neutral, p))
                })
                .when(auto, |el| {
                    el.child(with_tip(
                        div()
                            .id("repair-auto")
                            .child(chip("auto", ChipKind::Neutral, p)),
                        "Linked automatically from your voiceprint",
                    ))
                })
                .child(div().flex_1())
                .child(
                    Button::icon_only("repair-close", IconName::X.path(), "Close speaker repair")
                        .ghost()
                        .sm()
                        .on_click(cx.listener(|d, _, _, cx| {
                            d.repair = None;
                            cx.notify();
                        })),
                ),
        )
        .child(
            div()
                .pl(px(37.))
                .pr(space::S4)
                .pt(space::S1)
                .pb(px(10.))
                .border_b_1()
                .border_color(p.hairline)
                .font_family(FONT_MONO)
                .text_size(text::SM)
                .text_color(p.text_subtle)
                .child(vm::cluster_summary(cluster, &d.data.turns)),
        )
        .child(crate::ui::scroll::area(
            div()
                .id("repair-body")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap(px(14.))
                .px(space::S4)
                .pt(px(14.))
                .pb(space::S4)
                .children(body),
        ))
        .children(retry.map(|(w, e)| {
            div()
                .px(space::S4)
                .pb(px(10.))
                .flex()
                .flex_wrap()
                .items_center()
                .gap(space::S2)
                .text_size(text::SM)
                .text_color(p.danger_text)
                .child(crate::ui::icon(
                    IconName::TriangleAlert.path(),
                    theme::size::ICON_SM,
                ))
                .child(e)
                .child(
                    Button::new("repair-retry", "Try again")
                        .ghost()
                        .sm()
                        .disabled(busy)
                        .on_click(cx.listener(move |d, _, _, cx| {
                            d.write(cluster, w.clone(), close_on, cx)
                        })),
                )
        }))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .px(space::S4)
                .py(px(10.))
                .border_t_1()
                .border_color(p.hairline)
                .child(
                    Button::new(
                        "repair-commit",
                        if busy { busy_label.to_string() } else { label },
                    )
                    .primary()
                    .disabled(busy || !ready)
                    .spinner(busy)
                    .on_click(commit),
                )
                .when(unnamed_left > 0, |el| {
                    el.child(
                        div()
                            .font_family(FONT_MONO)
                            .text_size(text::SM)
                            .text_color(p.text_subtle)
                            .child(format!("{unnamed_left} more unnamed here")),
                    )
                }),
        );
    Some(
        panel
            .with_animation(
                ("repair-in", cluster as u64),
                Animation::new(motion::MED).with_easing(motion::expo),
                |el, t| el.opacity(t).right(px(-24. * (1. - t))),
            )
            .into_any_element(),
    )
}
