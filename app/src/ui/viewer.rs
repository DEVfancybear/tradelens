//! The picture viewer: one picture of a gallery full size, over the whole window.
//!
//! It is a layer of the window's root, like the search palette (not a `dialog()`, whose native
//! layer does not paint on Windows): a scrim that closes it, the picture fitted to the window
//! with its caption under it, a counter, previous / next, zoom with a drag to pan, and the two
//! ways out of the app (open with the default program, show in the folder).
//!
//! A gallery opens it with [`open`]; the window's root places [`overlay`] once, last. The
//! viewer reads the pictures through the [`Request`]'s `slides` closure, so it follows the list it
//! was opened from, and gives the keyboard focus back to the tile through `restore` when it
//! closes. Keys while it has the focus: Left / Right (previous / next, wrapping), Escape, `+` and
//! `-` (zoom), `0` (fit).

use std::{
    fmt,
    path::{Path, PathBuf},
    rc::Rc,
};

use nsh_base::{button, theme};
use nsh_core::prelude::*;

use super::{
    controls::{Variant, action, action_icon, icon_action},
    files::file_url,
    icons::{Icon, icon},
};

/// Room on each side of the picture for the arrows, in points.
const PAD_X: f32 = 72.0;
/// Room above and below the picture.
const PAD_Y: f32 = 8.0;
/// The most a picture is magnified, relative to the size that fits the window.
const MAX_ZOOM: f32 = 16.0;
/// Each `+` / `-` press and each notch of the wheel magnifies by this factor.
const ZOOM_STEP: f32 = 1.25;
/// The magnification the "100%" button gives a picture whose real size is not known, or is not
/// larger than the window.
const FALLBACK_ZOOM: f32 = 2.0;
/// A picture is shown "fit" up to this magnification.
const FIT_LIMIT: f32 = 1.001;
/// How much of the page shows through the scrim, from 0 (all) to 1 (none).
const SCRIM_OPACITY: f32 = 0.94;
/// A wheel of precise points (a touchpad) counts as one notch per this many points.
const POINTS_PER_NOTCH: f32 = 120.0;

/// One picture of the viewer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slide {
    /// The picture's file; `None` when the data folder is not known.
    pub file: Option<PathBuf>,
    /// The line under the picture: its timeframe and caption.
    pub caption: String,
    /// What else is known about it (when it was added); may be empty.
    pub detail: String,
}

/// The pictures the viewer reads, live.
pub type Slides = Rc<dyn Fn() -> Vec<Slide>>;

/// What a gallery asks the viewer to show.
#[derive(Clone)]
pub struct Request {
    label: &'static str,
    slides: Slides,
    start: usize,
    restore: Rc<dyn Fn(usize)>,
}

impl Request {
    /// A viewer of `slides` (read live), named `label` (`"Ảnh chart"`), opening on the picture
    /// at `start`. When it closes it calls `restore` with the index of the picture it was on, to
    /// put the keyboard focus back on that picture's tile.
    pub fn new(label: &'static str, slides: Slides, start: usize, restore: Rc<dyn Fn(usize)>) -> Self {
        Self { label, slides, start, restore }
    }
}

impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Request").field("label", &self.label).field("start", &self.start).finish_non_exhaustive()
    }
}

/// The viewer that is open, if one is.
#[derive(Clone, Default)]
struct Shown(Option<Request>);

fn shown() -> Signal<Shown> {
    global::<Shown>()
}

/// Opens the viewer on `request`'s picture.
pub fn open(request: Request) {
    shown().set(Shown(Some(request)));
}

/// Closes the viewer if it was opened from `slides` (a gallery that goes away takes its viewer
/// with it: what the viewer reads would be gone). The keyboard focus is not moved.
pub fn close_if_from(slides: &Slides) {
    let state = shown();
    if state.get_untracked().0.is_some_and(|request| Rc::ptr_eq(&request.slides, slides)) {
        state.set(Shown(None));
    }
}

/// Closes the viewer and gives the focus back to the tile of picture `index`.
fn dismiss(index: usize) {
    let state = shown();
    if let Some(request) = state.get_untracked().0 {
        state.set(Shown(None));
        (request.restore)(index);
    }
}

/// The viewer over the window while it is open; nothing while it is not. Place it as the last
/// child of the window's root.
pub fn overlay() -> impl IntoElement {
    let state = shown();
    when(move || state.get().0.is_some(), || untracked(layer))
}

/// Which neighbour a step goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Way {
    Previous,
    Next,
}

/// The picture after a step from `current` among `len`, wrapping round the ends.
fn neighbour(current: usize, len: usize, way: Way) -> usize {
    if len == 0 {
        return 0;
    }
    match way {
        Way::Previous => (current % len + len - 1) % len,
        Way::Next => (current % len + 1) % len,
    }
}

/// The two ways of handing the file to the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outside {
    /// The default program for the file.
    Open,
    /// The file manager, with the file selected.
    Reveal,
}

/// The frame of the area the picture lives in, in points, from its layout.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Stage {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

impl Stage {
    /// The room for the picture: the stage without its padding.
    fn holder(self) -> (f32, f32) {
        ((self.width - 2.0 * PAD_X).max(0.0), (self.height - 2.0 * PAD_Y).max(0.0))
    }

    /// Where a point of the window lies from the stage's centre.
    fn offset_from_centre(self, point: (f32, f32)) -> (f32, f32) {
        (point.0 - (self.x + self.width / 2.0), point.1 - (self.y + self.height / 2.0))
    }
}

/// The size of the picture when it fits `holder`, keeping its proportions: its real size is
/// `natural` when it is known, otherwise the picture is taken to fill the holder.
fn fit_box(holder: (f32, f32), natural: Option<(f32, f32)>) -> (f32, f32) {
    match natural {
        Some((width, height)) if width > 0.0 && height > 0.0 => {
            let scale = (holder.0 / width).min(holder.1 / height);
            (width * scale, height * scale)
        }
        _ => holder,
    }
}

/// The magnification at which one pixel of the picture is one point of the screen; a plain
/// [`FALLBACK_ZOOM`] when its real size is unknown or fits the window already.
fn actual_zoom(holder: (f32, f32), natural: Option<(f32, f32)>) -> f32 {
    let Some((width, _)) = natural else { return FALLBACK_ZOOM };
    let fitted = fit_box(holder, natural).0;
    if fitted <= 0.0 {
        return FALLBACK_ZOOM;
    }
    let ratio = width / fitted;
    if ratio > 1.0 + 0.05 { ratio.min(MAX_ZOOM) } else { FALLBACK_ZOOM }
}

/// How far the picture may be moved from the middle, each way, so that it never leaves a gap
/// at the edge it covers.
fn pan_limit(holder: (f32, f32), natural: Option<(f32, f32)>, zoom: f32) -> (f32, f32) {
    let shown = fit_box(holder, natural);
    (((shown.0 * zoom - holder.0) / 2.0).max(0.0), ((shown.1 * zoom - holder.1) / 2.0).max(0.0))
}

fn clamp_pan(pan: (f32, f32), limit: (f32, f32)) -> (f32, f32) {
    (pan.0.clamp(-limit.0, limit.0), pan.1.clamp(-limit.1, limit.1))
}

/// The pan after changing the magnification from `from` to `to` so that the point `about`
/// (from the centre) stays under the pointer.
fn pan_about(pan: (f32, f32), from: f32, to: f32, about: (f32, f32)) -> (f32, f32) {
    if from <= 0.0 {
        return pan;
    }
    let ratio = to / from;
    (about.0 - ratio * (about.0 - pan.0), about.1 - ratio * (about.1 - pan.1))
}

/// The magnification after `steps` presses of `+` (negative: of `-`).
fn stepped_zoom(zoom: f32, steps: f32) -> f32 {
    (zoom * ZOOM_STEP.powf(steps)).clamp(1.0, MAX_ZOOM)
}

/// The notches a wheel turned, positive for zooming in (the wheel pushed away from the user).
fn notches(delta: WheelDelta) -> f32 {
    let (_, y) = delta.xy();
    let turned = if delta.precise() { y / POINTS_PER_NOTCH } else { y };
    (-turned).clamp(-3.0, 3.0)
}

/// What the viewer remembers about the picture it shows.
#[derive(Clone, Copy)]
struct Frame {
    index: Signal<usize>,
    /// 1 fits the window.
    zoom: Signal<f32>,
    /// How far the picture was moved from the middle, in points.
    pan: Signal<(f32, f32)>,
    stage: Signal<Stage>,
    /// The picture's size in pixels, once known.
    natural: Signal<Option<(f32, f32)>>,
    /// A line about what went wrong.
    notice: Signal<Option<String>>,
}

impl Frame {
    fn new(start: usize) -> Self {
        Self {
            index: signal(start),
            zoom: signal(1.0),
            pan: signal((0.0, 0.0)),
            stage: signal(Stage::default()),
            natural: signal(None),
            notice: signal(None),
        }
    }

    /// Magnifies to `zoom`, keeping the point `about` (from the centre) where it is.
    fn zoom_to(self, zoom: f32, about: (f32, f32)) {
        let from = self.zoom.get_untracked();
        let to = zoom.clamp(1.0, MAX_ZOOM);
        let pan = pan_about(self.pan.get_untracked(), from, to, about);
        let limit = pan_limit(self.stage.get_untracked().holder(), self.natural.get_untracked(), to);
        self.pan.set(clamp_pan(pan, limit));
        self.zoom.set(to);
    }

    fn zoom_by(self, steps: f32, about: (f32, f32)) {
        self.zoom_to(stepped_zoom(self.zoom.get_untracked(), steps), about);
    }

    /// Fit to the window, or the picture's real size when it fits already.
    fn toggle(self) {
        let target = if self.zoom.get_untracked() > FIT_LIMIT {
            1.0
        } else {
            actual_zoom(self.stage.get_untracked().holder(), self.natural.get_untracked())
        };
        self.zoom_to(target, (0.0, 0.0));
    }

    /// Moves the picture by a drag of `by` points.
    fn drag_by(self, by: (f32, f32)) {
        let limit =
            pan_limit(self.stage.get_untracked().holder(), self.natural.get_untracked(), self.zoom.get_untracked());
        let pan = self.pan.get_untracked();
        self.pan.set(clamp_pan((pan.0 + by.0, pan.1 + by.1), limit));
    }

    /// The percentage the readout says: the picture's pixels per point when its size is known,
    /// otherwise the magnification over the fitted picture.
    fn percent(self) -> f32 {
        let zoom = self.zoom.get();
        let fitted = fit_box(self.stage.get().holder(), self.natural.get()).0;
        match self.natural.get() {
            Some((width, _)) if width > 0.0 => zoom * fitted / width * 100.0,
            _ => zoom * 100.0,
        }
    }

    /// Starts reading the real size of the picture at `file`; the answer is dropped if the
    /// viewer moved on meanwhile.
    fn measure(self, file: Option<&Path>, index: usize) {
        self.natural.set(None);
        let Some(url) = file.and_then(file_url) else { return };
        spawn(async move {
            let Ok(size) = image_size(url).await else { return };
            if self.index.try_get() != Some(index) {
                return;
            }
            if let (Ok(width), Ok(height)) = (u16::try_from(size.width.0), u16::try_from(size.height.0)) {
                self.natural.try_set(Some((f32::from(width), f32::from(height))));
            }
        });
    }
}

/// The viewer: the scrim, the header, the stage with the picture, and the caption.
fn layer() -> View {
    let Some(request) = shown().get_untracked().0 else { return view() };
    let Request { label, slides, start, .. } = request;
    let list = computed(move || slides());
    let frame = Frame::new(start);
    let root = element_ref();
    // The focus moves into the viewer once it is on screen, so its keys work at once.
    request_animation_frame(move |_| root.focus());

    let count = move || list.with(Vec::len);
    // The list may shrink under the viewer (a sync): it shows the last picture then.
    let at = move || frame.index.get().min(count().saturating_sub(1));
    let current = computed(move || list.with(|l| l.get(at()).cloned()));
    let show = move |to: usize| {
        frame.index.set(to);
        frame.zoom.set(1.0);
        frame.pan.set((0.0, 0.0));
        let file = list.with(|l| l.get(to).and_then(|s| s.file.clone()));
        frame.measure(file.as_deref(), to);
    };
    show(start.min(count().saturating_sub(1)));
    let step = move |way: Way| {
        let len = count();
        if len > 1 {
            show(neighbour(frame.index.get_untracked().min(len - 1), len, way));
        }
    };
    let close = move || dismiss(frame.index.get_untracked().min(count().saturating_sub(1)));
    let zoom_in = move || frame.zoom_to(actual_zoom_of(frame), (0.0, 0.0));
    let outside = move |how: Outside| {
        let Some(path) = current.get_untracked().and_then(|slide| slide.file) else {
            frame.notice.set(Some("Không tìm thấy file ảnh.".to_owned()));
            return;
        };
        spawn(async move {
            let done = match how {
                Outside::Open => open_path(path).await,
                Outside::Reveal => reveal_path(path).await,
            };
            frame.notice.try_set(done.err().map(|error| format!("Không mở được ảnh: {error}")));
        });
    };
    let fit = move || frame.zoom.get() <= FIT_LIMIT;
    // Keys of the window, not of the focused view: a press on one of the buttons moves the focus
    // off the viewer, and the keys must keep working. They live as long as the viewer does.
    on_key("escape", move |_| close());
    on_key("left", move |_| step(Way::Previous));
    on_key("right", move |_| step(Way::Next));
    for stroke in ["=", "shift+=", "mod+="] {
        on_key(stroke, move |_| frame.zoom_by(1.0, (0.0, 0.0)));
    }
    for stroke in ["-", "shift+-", "mod+-"] {
        on_key(stroke, move |_| frame.zoom_by(-1.0, (0.0, 0.0)));
    }
    on_key("0", move |_| frame.zoom_to(1.0, (0.0, 0.0)));

    view()
        .absolute()
        .inset_0()
        .focusable(true)
        .element_ref(&root)
        .on_wheel(move |event: &WheelEvent| {
            let about =
                frame.stage.get_untracked().offset_from_centre((event.page_position.x.0, event.page_position.y.0));
            frame.zoom_by(notches(event.delta), about);
        })
        .child(
            button()
                .cursor_pointer()
                .on_press(close)
                .accessibility_label("Đóng trình xem ảnh")
                .absolute()
                .inset_0()
                .bg(move || theme().background)
                .opacity(SCRIM_OPACITY),
        )
        .child(header(label, frame, count, fit, close, outside))
        .child(stage(frame, current, count, fit, step, zoom_in))
        .child(footer(frame, current))
}

/// The magnification of the "100%" press.
fn actual_zoom_of(frame: Frame) -> f32 {
    actual_zoom(frame.stage.get_untracked().holder(), frame.natural.get_untracked())
}

/// The bar above the picture: what it is, where it stands, zoom, and the way out.
fn header(
    label: &'static str,
    frame: Frame,
    count: impl Fn() -> usize + Copy + 'static,
    fit: impl Fn() -> bool + Copy + 'static,
    close: impl Fn() + Copy + 'static,
    outside: impl Fn(Outside) + Copy + 'static,
) -> impl IntoElement {
    view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_2()
        .px_4()
        .py_3()
        .pointer_events(PointerEvents::BoxNone)
        .child(text(label).text_sm().font_weight(FontWeight::SEMIBOLD))
        .child(
            text(move || format!("{} / {}", frame.index.get().min(count().saturating_sub(1)) + 1, count()))
                .text_sm()
                .text_color(move || theme().muted_foreground),
        )
        .child(view().flex_1().min_w(px(0.0)))
        .child(icon_action(Icon::Minus, "Thu nhỏ", Variant::Secondary, move || frame.zoom_by(-1.0, (0.0, 0.0))))
        .child(
            text(move || format!("{:.0}%", frame.percent()))
                .text_sm()
                .w(px(48.0))
                .text_center()
                .text_color(move || theme().muted_foreground),
        )
        .child(icon_action(Icon::Plus, "Phóng to", Variant::Secondary, move || frame.zoom_by(1.0, (0.0, 0.0))))
        .child(action(move || if fit() { "100%" } else { "Vừa khung" }, Variant::Secondary, move || frame.toggle()))
        .child(action_icon(Icon::Share, "Mở bằng ứng dụng khác", Variant::Secondary, move || {
            outside(Outside::Open);
        }))
        .child(action_icon(Icon::Folder, "Hiện trong thư mục", Variant::Secondary, move || {
            outside(Outside::Reveal);
        }))
        .child(icon_action(Icon::Close, "Đóng ảnh", Variant::Secondary, close))
}

/// The area the picture fills, with the arrows at its sides.
fn stage(
    frame: Frame,
    current: Memo<Option<Slide>>,
    count: impl Fn() -> usize + Copy + 'static,
    fit: impl Fn() -> bool + Copy + 'static,
    step: impl Fn(Way) + Copy + 'static,
    zoom_in: impl Fn() + Copy + 'static,
) -> impl IntoElement {
    let arrow = move |glyph: Icon, label: &'static str, way: Way| {
        // A strip over the whole side that takes no clicks itself: the scrim behind still hears them.
        let strip = view().absolute().top_0().bottom_0().justify_center().pointer_events(PointerEvents::BoxNone);
        let strip = match way {
            Way::Previous => strip.left(px(16.0)),
            Way::Next => strip.right(px(16.0)),
        };
        when(
            move || count() > 1,
            move || strip.clone().child(icon_action(glyph, label, Variant::Secondary, move || step(way))),
        )
    };
    view()
        .flex_1()
        .min_h(px(0.0))
        .overflow_hidden()
        .pointer_events(PointerEvents::BoxNone)
        .on_layout(move |bounds: &Bounds<Points>| {
            frame.stage.set(Stage {
                x: bounds.origin.x.0,
                y: bounds.origin.y.0,
                width: bounds.size.width.0,
                height: bounds.size.height.0,
            });
        })
        .child(
            view()
                .absolute()
                .inset_0()
                .px(px(PAD_X))
                .py(px(PAD_Y))
                .items_center()
                .justify_center()
                .pointer_events(PointerEvents::BoxNone)
                .child(dynamic(move || {
                    // Only the shown picture decides what is built; what building it reads is not tracked.
                    let slide = current.get();
                    untracked(|| slide.map(|slide| picture(&slide, frame, fit, zoom_in)))
                })),
        )
        .child(arrow(Icon::ChevronLeft, "Ảnh trước", Way::Previous))
        .child(arrow(Icon::ChevronRight, "Ảnh sau", Way::Next))
}

/// The picture at its place: the box it fits, moved by the pan, scaled by the zoom, with the
/// layer that takes the press (zoom in) or the drag (pan).
fn picture(
    slide: &Slide,
    frame: Frame,
    fit: impl Fn() -> bool + Copy + 'static,
    zoom_in: impl Fn() + Copy + 'static,
) -> View {
    let fitted = move || fit_box(frame.stage.get().holder(), frame.natural.get());
    let name: SharedString = format!("Ảnh {}", slide.caption).into();
    let url = slide.file.as_deref().and_then(file_url);
    let last = signal(None::<(f32, f32)>);
    view()
        .w(move || px(fitted().0))
        .h(move || px(fitted().1))
        .translate_x(move || px(frame.pan.get().0))
        .translate_y(move || px(frame.pan.get().1))
        .pointer_events(PointerEvents::BoxNone)
        .child(
            view()
                .w_full()
                .h_full()
                .scale(move || frame.zoom.get())
                .items_center()
                .justify_center()
                .child(icon(Icon::Image, 32.0).text_color(move || theme().muted_foreground))
                .child(url.map(|url| image(url).absolute().inset_0().object_fit(ResizeMode::Contain).label(name)))
                .child(
                    when(fit, move || view().absolute().inset_0().cursor(Cursor::ZoomIn).on_press(zoom_in)).otherwise(
                        move || {
                            view()
                                .absolute()
                                .inset_0()
                                .cursor(Cursor::Grab)
                                .on_mouse_move(move |event: &MouseEvent| {
                                    let here = (event.page_position.x.0, event.page_position.y.0);
                                    if event.pressed_button.is_none() {
                                        last.set(None);
                                        return;
                                    }
                                    if let Some(from) = last.get_untracked() {
                                        frame.drag_by((here.0 - from.0, here.1 - from.1));
                                    }
                                    last.set(Some(here));
                                })
                                .on_mouse_leave(move |_: &MouseEvent| last.set(None))
                        },
                    ),
                ),
        )
}

/// The caption of the picture, what else is known of it, the keys, and a problem if there is one.
fn footer(frame: Frame, current: Memo<Option<Slide>>) -> impl IntoElement {
    let line = move |read: fn(&Slide) -> &String| {
        move || current.with(|slide| slide.as_ref().map_or_default(|s| read(s).clone()))
    };
    view()
        .items_center()
        .gap_1()
        .px_4()
        .py_3()
        .pointer_events(PointerEvents::BoxNone)
        .child(text(line(|s| &s.caption)).text_sm().font_weight(FontWeight::MEDIUM))
        .child(text(line(|s| &s.detail)).text_xs().text_color(move || theme().muted_foreground))
        .child(dynamic(move || frame.notice.get().map(|notice| text(notice).text_xs().text_color(super::theme::loss))))
        .child(
            text("← → chuyển ảnh   + − phóng to / thu nhỏ   0 vừa khung   Esc đóng")
                .text_xs()
                .text_color(move || theme().muted_foreground),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stepping_wraps_round_the_ends() {
        assert_eq!(neighbour(0, 3, Way::Next), 1);
        assert_eq!(neighbour(2, 3, Way::Next), 0, "past the last is the first");
        assert_eq!(neighbour(0, 3, Way::Previous), 2, "before the first is the last");
        assert_eq!(neighbour(5, 3, Way::Previous), 1, "a stale index is taken modulo the length");
        assert_eq!(neighbour(0, 0, Way::Next), 0, "nothing to step over");
    }

    #[test]
    fn a_picture_fits_the_holder_keeping_its_proportions() {
        assert_eq!(fit_box((800.0, 400.0), Some((1600.0, 1600.0))), (400.0, 400.0), "limited by the height");
        assert_eq!(fit_box((800.0, 400.0), Some((1600.0, 400.0))), (800.0, 200.0), "limited by the width");
        assert_eq!(fit_box((800.0, 400.0), None), (800.0, 400.0), "unknown fills the holder");
        assert_eq!(fit_box((800.0, 400.0), Some((0.0, 10.0))), (800.0, 400.0), "a bad size is unknown");
    }

    #[test]
    fn the_real_size_is_one_pixel_per_point_or_a_plain_magnification() {
        let holder = (800.0, 400.0);
        assert!((actual_zoom(holder, Some((1600.0, 800.0))) - 2.0).abs() < 1e-4, "twice the fitted size");
        assert!((actual_zoom(holder, Some((4000.0, 2000.0))) - 5.0).abs() < 1e-4);
        assert!((actual_zoom(holder, Some((300.0, 200.0))) - FALLBACK_ZOOM).abs() < 1e-4, "smaller than the window");
        assert!((actual_zoom(holder, None) - FALLBACK_ZOOM).abs() < 1e-4);
    }

    #[test]
    fn the_pan_cannot_leave_a_gap() {
        let holder = (800.0, 400.0);
        assert_eq!(pan_limit(holder, None, 1.0), (0.0, 0.0), "nothing to move when it fits");
        assert_eq!(pan_limit(holder, None, 2.0), (400.0, 200.0), "half of the extra on each side");
        assert_eq!(clamp_pan((900.0, -300.0), (400.0, 200.0)), (400.0, -200.0));
    }

    #[test]
    fn zooming_keeps_the_point_under_the_pointer() {
        // Zooming about the centre moves nothing.
        assert_eq!(pan_about((0.0, 0.0), 1.0, 2.0, (0.0, 0.0)), (0.0, 0.0));
        // A point 100 points right of the centre stays there: the picture slides left by 100.
        assert_eq!(pan_about((0.0, 0.0), 1.0, 2.0, (100.0, 0.0)), (-100.0, 0.0));
        // And back again.
        assert_eq!(pan_about((-100.0, 0.0), 2.0, 1.0, (100.0, 0.0)), (0.0, 0.0));
    }

    #[test]
    fn the_zoom_stays_between_fit_and_the_maximum() {
        assert!((stepped_zoom(1.0, -1.0) - 1.0).abs() < 1e-6, "no smaller than the window");
        assert!((stepped_zoom(1.0, 1.0) - ZOOM_STEP).abs() < 1e-6);
        assert!((stepped_zoom(MAX_ZOOM, 3.0) - MAX_ZOOM).abs() < 1e-6);
    }

    #[test]
    fn a_wheel_zooms_in_when_pushed_away() {
        assert!(notches(WheelDelta::Lines(Point::new(0.0, -1.0))) > 0.0);
        assert!(notches(WheelDelta::Lines(Point::new(0.0, 1.0))) < 0.0);
        assert!((notches(WheelDelta::Points(Point::new(Points(0.0), Points(-120.0)))) - 1.0).abs() < 1e-6);
        assert!((notches(WheelDelta::Lines(Point::new(0.0, -50.0))) - 3.0).abs() < 1e-6, "one event is capped");
    }
}
