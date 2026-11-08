//! `DrawerStatePanels` + `DrawerNotice`: every non-reader state, each with a
//! headline, a plain sentence, the way out and the provenance footnote; loading
//! is a skeleton in the shape of the content (three gutter + prose rows).

use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context};

use super::view_model::{self as vm, Panel};
use super::AudioDrawer;
use crate::theme::{self, space, text, Palette, FONT_MONO};
use crate::ui::Button;

fn notice(
    title: &str,
    body: String,
    acts: Vec<AnyElement>,
    foot: String,
    p: &Palette,
) -> AnyElement {
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(space::S2)
        .p(space::S5)
        .child(
            div()
                .text_size(text::MD)
                .font_weight(theme::W_SEMIBOLD)
                .text_color(p.text_strong)
                .child(title.to_string()),
        )
        .child(
            div()
                .max_w(px(300.))
                .text_center()
                .text_size(text::SM)
                .line_height(px(11. * 1.6))
                .text_color(p.text_muted)
                .child(body),
        )
        .when(!acts.is_empty(), |el| {
            el.child(
                div()
                    .mt(space::S1)
                    .flex()
                    .flex_wrap()
                    .justify_center()
                    .gap(space::S2)
                    .children(acts),
            )
        })
        .when(!foot.is_empty(), |el| {
            el.child(
                div()
                    .font_family(FONT_MONO)
                    .text_size(text::XS)
                    .text_color(p.text_subtle)
                    .child(foot),
            )
        })
        .into_any_element()
}

fn skeleton(p: &Palette) -> AnyElement {
    let bar = |w: f32| {
        div()
            .h(px(9.))
            .w(gpui_kit::relative(w))
            .rounded(px(3.))
            .bg(p.surface_hover)
    };
    div()
        .id("drawer-skeleton")
        .role(gpui_kit::Role::Status)
        .aria_label("Loading transcript")
        .flex_1()
        .min_h_0()
        .overflow_hidden()
        .px(space::S5)
        .py(space::S4)
        .children([3usize, 2, 3].map(|lines| {
            div()
                .flex()
                .gap(space::S4)
                .mb(space::S4)
                .child(div().w(px(110.)).flex().justify_end().child(bar(0.7)))
                .child(
                    div()
                        .w(px(300.))
                        .flex()
                        .flex_col()
                        .gap(px(7.))
                        .children([1., 0.92, 0.64].into_iter().take(lines).map(bar)),
                )
        }))
        .into_any_element()
}

pub fn render(
    d: &AudioDrawer,
    panel: Panel,
    p: &Palette,
    cx: &mut Context<AudioDrawer>,
) -> AnyElement {
    let duration = vm::player_time(d.segment.duration_s().round());
    let provenance = vm::provenance_footnote(d.data.provenance.as_ref());
    let rerun_disabled = d.rerun_loading;
    let rerun = |id: &'static str, label: &str, ghost: bool, cx: &mut Context<AudioDrawer>| {
        let b = Button::new(id, label.to_string())
            .sm()
            .disabled(rerun_disabled);
        let b = if ghost { b.ghost() } else { b };
        b.on_click(cx.listener(|d, _, _, cx| d.rerun(cx)))
            .into_any_element()
    };
    match panel {
        Panel::Reader | Panel::Skeleton => skeleton(p),
        Panel::Processing => {
            let foot = d
                .data
                .pending
                .as_ref()
                .map(|j| {
                    let stage = vm::stage_label(d.segment.source, &j.processor);
                    let ago = queued_ago(&j.queued_at, crate::ui::clock::now_ms(cx));
                    [stage, ago].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ")
                })
                .unwrap_or_default();
            notice(
                "Still working on this one",
                "Speech detection runs first, then transcription, then speaker analysis. The text appears as each stage lands — you don't have to wait here.".into(),
                Vec::new(),
                foot,
                p,
            )
        }
        Panel::NoSpeech => notice(
            "No speech in this segment",
            format!("{duration} of room tone. Nothing was transcribed because there was nothing to transcribe."),
            vec![
                Button::new("drawer-play-anyway", "Play it anyway")
                    .sm()
                    .on_click(cx.listener(|d, _, _, cx| d.toggle(cx)))
                    .into_any_element(),
                rerun("drawer-rerun-panel", "Rerun analysis", true, cx),
            ],
            if provenance.is_empty() { d.data.speakers_notice.clone().unwrap_or_default() } else { provenance },
            p,
        ),
        Panel::Failed => notice(
            "This segment could not be transcribed",
            d.data.error.clone().unwrap_or("The transcription job failed.".into()),
            vec![rerun(
                "drawer-rerun-panel",
                if d.rerun_loading { "Retrying…" } else { "Retry transcription" },
                false,
                cx,
            )],
            provenance,
            p,
        ),
        Panel::SpeakersFailed => notice(
            "The words are here. The speakers aren't.",
            d.data.speakers_error.clone().unwrap_or(
                "Transcription finished; speaker analysis stopped partway through. You can read the transcript now and retry the speaker pass.".into(),
            ),
            vec![
                Button::new("drawer-read-anyway", "Read without speakers")
                    .sm()
                    .on_click(cx.listener(|d, _, _, cx| {
                        d.ignore_speaker_failure = true;
                        cx.notify();
                    }))
                    .into_any_element(),
                Button::new(
                    "drawer-retry-speakers",
                    if d.speaker_retry_loading { "Retrying…" } else { "Retry speaker analysis" },
                )
                .sm()
                .ghost()
                .disabled(d.speaker_retry_loading || d.data.speakers_running)
                .on_click(cx.listener(|d, _, _, cx| d.retry_speakers(cx)))
                .into_any_element(),
            ],
            provenance,
            p,
        ),
        Panel::NotRun => notice(
            "Speaker analysis hasn't run for this segment",
            "It was captured before this pass was turned on. Running it now costs about ten seconds and changes nothing else.".into(),
            vec![rerun(
                "drawer-rerun-panel",
                if d.rerun_loading { "Starting…" } else { "Run speaker analysis" },
                false,
                cx,
            )],
            String::new(),
            p,
        ),
    }
}

/// `queued 40s ago` from the job's ISO time; "" when it won't parse.
fn queued_ago(iso: &str, now_ms: i64) -> String {
    use time::format_description::well_known::Rfc3339;
    time::OffsetDateTime::parse(iso, &Rfc3339)
        .map(|t| {
            let at = (t.unix_timestamp_nanos() / 1_000_000) as i64;
            format!("queued {} ago", vm::compact_duration(now_ms - at))
        })
        .unwrap_or_default()
}
