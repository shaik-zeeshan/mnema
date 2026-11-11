//! The subject page's frame peek (`SubjectDetail.svelte` `onViewInTimeline`):
//! an evidence row whose activity cites a frame opens Chat's frame peek in
//! place (`chat::peek`, the one `FrameDetailModal`), "show this frame in the
//! timeline" inside it; a spoken one opens its segment in the Timeline; one
//! with nothing to show says it couldn't open.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::{Context, Entity, FocusHandle, Pixels, Size, Window};

use crate::chat::peek::{PeekData, PeekHost, PeekOps as _};
use crate::chat::source_data::{CoreSources, NoSources, SourceBackend};
use crate::chat::Source;
use crate::insights::Insights;
use crate::timeline::image_cache::FrameImages;
use crate::ui::toast::Toast;

pub struct Peek {
    pub src: Option<Source>,
    pub data: PeekData,
    gen: u64,
    focus: FocusHandle,
    hero: Rc<Cell<Size<Pixels>>>,
    images: Entity<FrameImages>,
    pub sources: Arc<dyn SourceBackend>,
}

impl Peek {
    pub fn new(cx: &mut Context<Insights>) -> Self {
        let hero: Rc<Cell<Size<Pixels>>> = Rc::default();
        Self {
            src: None,
            data: PeekData::default(),
            gen: 0,
            focus: cx.focus_handle(),
            images: FrameImages::new(hero.clone(), cx),
            hero,
            sources: match crate::chat::core(cx) {
                Some(core) => Arc::new(CoreSources(core)),
                None => Arc::new(NoSources),
            },
        }
    }
}

impl PeekHost for Insights {
    fn peek_src(&self) -> Option<&Source> {
        self.peek.src.as_ref()
    }
    fn set_peek_src(&mut self, src: Option<Source>) {
        self.peek.src = src;
    }
    fn peek_data(&self) -> &PeekData {
        &self.peek.data
    }
    fn peek_data_mut(&mut self) -> &mut PeekData {
        &mut self.peek.data
    }
    fn peek_gen(&mut self) -> &mut u64 {
        &mut self.peek.gen
    }
    fn peek_focus(&self) -> &FocusHandle {
        &self.peek.focus
    }
    fn hero_size(&self) -> &Rc<Cell<Size<Pixels>>> {
        &self.peek.hero
    }
    fn peek_images(&self) -> &Entity<FrameImages> {
        &self.peek.images
    }
    fn peek_sources(&self) -> Arc<dyn SourceBackend> {
        self.peek.sources.clone()
    }
}

fn iso(ms: i64) -> String {
    time::OffsetDateTime::from_unix_timestamp_nanos(ms as i128 * 1_000_000)
        .ok()
        .and_then(|t| {
            t.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
        .unwrap_or_default()
}

impl Insights {
    /// An evidence row: its activity's first cited moment.
    pub fn view_evidence(&mut self, activity: i64, window: &mut Window, cx: &mut Context<Self>) {
        let first = self
            .detail
            .as_ref()
            .and_then(|d| d.activities.get(&activity))
            .and_then(|a| a.evidence.first().cloned().map(|r| (r, a.started_at_ms)));
        match first {
            Some((r, started)) if r.subject_type == "frame" => {
                let at = iso(r.captured_at_ms.unwrap_or(started));
                self.peek_frame(
                    Source {
                        kind: "frame".into(),
                        frame_id: Some(r.subject_id),
                        started_at: at.clone(),
                        ended_at: at,
                        ..Default::default()
                    },
                    window,
                    cx,
                );
            }
            Some((r, _)) if r.subject_type == "audio_segment" => {
                let id = r.subject_id;
                cx.defer(move |cx| crate::timeline::open_audio(id, 0, None, None, cx));
            }
            _ => {
                let t = Toast::danger("Couldn't open that moment in the Timeline");
                self.toasts.update(cx, |ts, cx| ts.push(t, cx));
            }
        }
    }
}
