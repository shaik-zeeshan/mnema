//! An app icon drawn crisp at any size. GPUI samples images bilinear with no
//! mipmaps, and `resolve_app_icons` writes 1024 px PNGs (AppKit's first rep),
//! so a plain `img(path)` squeezed into a 28 device-px box aliases into mush.
//! Here each (icon, device px) is decoded once, area-averaged down to the box.
//! (The 1024 px PNG is also a 4 MB atlas tile per icon; the fit one is ~3 KB.)

use std::future::Future;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::{
    img, px, AnyElement, App, Asset, Div, ImageCacheError, ImageSource, Img, RenderImage,
    SharedString,
};
use image::{imageops, Frame, RgbaImage};

/// Where an icon's encoded bytes live.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub enum Src {
    /// A cached PNG from `resolve_app_icons`.
    File(PathBuf),
    /// An icon compiled into the binary (Mnema's own).
    Bundled(&'static [u8]),
}

/// The icon decoded to `(src, side in device px)`.
pub(crate) struct Fit;

impl Asset for Fit {
    type Source = (Src, u32);
    type Output = Result<Arc<RenderImage>, ImageCacheError>;

    fn load(
        (src, side): Self::Source,
        _: &mut App,
    ) -> impl Future<Output = Self::Output> + Send + 'static {
        async move {
            let file;
            let bytes = match &src {
                Src::File(path) => {
                    file = std::fs::read(path).map_err(|e| ImageCacheError::Io(Arc::new(e)))?;
                    &file[..]
                }
                Src::Bundled(bytes) => bytes,
            };
            decode(bytes, side).map(Arc::new)
        }
    }
}

/// Decode `bytes` to fit a `side`² box (never upscaled), as GPUI's BGRA.
fn decode(bytes: &[u8], side: u32) -> Result<RenderImage, ImageCacheError> {
    let err = |e: image::ImageError| ImageCacheError::Other(Arc::new(e.into()));
    let mut rgba = image::load_from_memory(bytes).map_err(err)?.into_rgba8();
    let (w, h) = rgba.dimensions();
    if w.max(h) > side {
        let k = side as f32 / w.max(h) as f32;
        let (tw, th) = (
            ((w as f32 * k).round() as u32).max(1),
            ((h as f32 * k).round() as u32).max(1),
        );
        // Average premultiplied: the transparent margin's black would darken the rim.
        premultiply(&mut rgba, true);
        rgba = imageops::thumbnail(&rgba, tw, th);
        premultiply(&mut rgba, false);
    }
    for px in rgba.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    Ok(RenderImage::new([Frame::new(rgba)]))
}

/// Multiply colour by alpha (`on`), or divide it back out.
fn premultiply(rgba: &mut RgbaImage, on: bool) {
    for px in rgba.pixels_mut() {
        let a = u32::from(px[3]);
        for c in &mut px.0[..3] {
            let v = u32::from(*c);
            *c = match (on, a) {
                (true, _) => (v * a + 127) / 255,
                (false, 0) => 0,
                (false, _) => ((v * 255 + a / 2) / a).min(255),
            } as u8;
        }
    }
}

/// The device-pixel side a `size` pt box needs at `scale`.
pub(crate) fn device_side(size: f32, scale: f32) -> u32 {
    (size * scale).ceil().max(1.) as u32
}

/// A `size` pt square icon, decoded for the window's scale factor.
pub fn app_icon(src: Src, size: f32) -> Img {
    img(ImageSource::Custom(Arc::new(move |window, cx| {
        let side = device_side(size, window.scale_factor());
        window.use_asset::<Fit>(&(src.clone(), side), cx)
    })))
    .flex_none()
    .size(px(size))
}

/// `appIconFallback`: the name's first letter, upper-cased, or "?".
pub fn initial(name: &str) -> String {
    name.trim()
        .chars()
        .next()
        .map_or_else(|| "?".into(), |c| c.to_uppercase().to_string())
}

/// `path`'s icon at `size`, or `tile` (a styled box) holding `name`'s initial
/// when there is no path, while the file decodes, and if it fails to.
pub fn icon_or_initial(
    path: Option<PathBuf>,
    name: &str,
    size: f32,
    tile: impl Fn() -> Div + 'static,
) -> AnyElement {
    let letter = SharedString::from(initial(name));
    let tile = Rc::new(move || tile().child(letter.clone()).into_any_element());
    match path {
        None => tile(),
        Some(path) => {
            let (loading, failed) = (tile.clone(), tile);
            app_icon(Src::File(path), size)
                .with_loading(move || loading())
                .with_fallback(move || failed())
                .into_any_element()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{div, Context, Render, TestAppContext, Window};

    struct Host(Src);

    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().child(app_icon(self.0.clone(), 14.))
        }
    }

    /// A 1024 px icon in a 14 pt box on a 2x window is decoded to exactly
    /// 28 px: never handed to GPUI full size (the Retina blur).
    #[gpui_kit::test]
    fn an_icon_is_decoded_at_its_box_times_the_scale_factor(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.png");
        image::RgbaImage::from_pixel(1024, 1024, image::Rgba([200, 40, 40, 255]))
            .save(&path)
            .unwrap();
        let src = Src::File(path);
        let (_, cx) = cx.add_window_view(|_, _| Host(src.clone()));
        cx.run_until_parked();
        cx.update(|window, cx| {
            assert_eq!(window.scale_factor(), 2.);
            // Already loaded: the element asked for this exact key.
            let image = window
                .get_asset::<Fit>(&(src, 28), cx)
                .expect("the element requested the 28 px decode")
                .unwrap();
            let s = image.size(0);
            assert_eq!((s.width.0, s.height.0), (28, 28));
        });
    }

    /// Half-transparent black margins don't darken an opaque icon's edge.
    #[test]
    fn the_downscale_averages_premultiplied() {
        let mut big = RgbaImage::from_pixel(4, 2, image::Rgba([0, 0, 0, 0]));
        for y in 0..2 {
            big.put_pixel(0, y, image::Rgba([0, 0, 255, 255]));
        }
        let mut png = std::io::Cursor::new(Vec::new());
        big.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let image = decode(png.get_ref(), 2).unwrap();
        // Left output pixel = 1 blue + 1 clear: half alpha, still full blue (BGRA).
        assert_eq!(&image.as_bytes(0).unwrap()[..4], &[255, 0, 0, 128]);
    }

    /// Draws `icon_or_initial` and notes whether the initial's tile was drawn.
    struct Either(Option<PathBuf>, std::rc::Rc<std::cell::Cell<bool>>);

    impl Render for Either {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let drawn = self.1.clone();
            drawn.set(false);
            div().child(icon_or_initial(self.0.clone(), "Notes", 13., move || {
                drawn.set(true);
                div()
            }))
        }
    }

    fn tile_drawn(cx: &mut TestAppContext, path: Option<PathBuf>) -> bool {
        let drawn = std::rc::Rc::new(std::cell::Cell::new(false));
        let (view, cx) = cx.add_window_view(|_, _| Either(path, drawn.clone()));
        cx.run_until_parked();
        view.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        drawn.get()
    }

    /// The initial stands in with no path and for a file that can't decode;
    /// a decoded icon replaces it.
    #[gpui_kit::test]
    fn the_initial_shows_until_an_icon_decodes(cx: &mut TestAppContext) {
        assert!(tile_drawn(cx, None));
        assert!(tile_drawn(cx, Some(PathBuf::from("/nonexistent/x.png"))));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ok.png");
        image::RgbaImage::from_pixel(64, 64, image::Rgba([0, 120, 0, 255]))
            .save(&path)
            .unwrap();
        assert!(!tile_drawn(cx, Some(path)));
    }

    #[test]
    fn the_initial_is_the_trimmed_first_letter_or_a_question_mark() {
        assert_eq!(initial(" safari"), "S");
        assert_eq!(initial("Zen Browser"), "Z");
        assert_eq!(initial("  "), "?");
    }

    #[test]
    fn device_side_rounds_up() {
        assert_eq!(device_side(14., 2.), 28);
        assert_eq!(device_side(15., 1.5), 23);
        assert_eq!(device_side(18., 1.), 18);
    }
}
