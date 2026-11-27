//! The picture gallery of an entry (a trade, a missed trade, a setup): a wrapping grid of
//! thumbnails with a caption, ending in the "Add pictures" tile that takes several files from the file
//! dialog, from a drop on the gallery, or from Ctrl+V.
//!
//! Pressing a thumbnail (or Enter on it, or its "View" button) opens the picture full size in the
//! [viewer](super::viewer). The tile's actions sit over its top-right corner: a handle to reorder
//! (drag, or the arrow keys), always there, and "Xem" and the remove button, which show while the
//! pointer is over the tile or the keyboard focus is in it. A removed picture is gone at once and
//! can be brought back for a few seconds from the line under the grid.
//!
//! The gallery edits a list of [`Attachment`]s it does not own: every change is reported whole
//! through `on_change`, and saving the list is the caller's job. New files are copied into the
//! data folder with `tradelens_core::attachments::import` on a blocking thread first.

use std::{cell::RefCell, path::PathBuf, rc::Rc, time::Duration};

use nsh_base::{button, theme};
use nsh_core::prelude::*;
use tradelens_core::attachments::{self, AttachError, Attachment, Source};

use super::{
    controls::link,
    files::{Accept, Pick, image_slot, pasted_images, pick_files, sift},
    icons::{Icon, icon},
    motion,
    parts::error_note,
    reorder::{Step, drag_handle, grid_columns, grid_drop_index, stepped},
    viewer::{self, Request, Slide, Slides},
};

/// Width of a tile, in points.
const TILE_WIDTH: f32 = 196.0;
/// Height of a tile's picture.
const TILE_IMAGE: f32 = 120.0;
/// Room under the picture for the caption row.
const TILE_CAPTION: f32 = 30.0;
/// Space between tiles.
const GAP: f32 = 12.0;
/// How long a removed picture can be brought back.
const UNDO_WINDOW: Duration = Duration::from_secs(6);
/// The size of the small buttons over a tile's corner.
const CHIP: f32 = 24.0;

/// The gallery of `items`, named `label` (`"Chart pictures"`) for screen readers, whose tiles read
/// "label 1", "label 2". `items` is read where it is bound, so it follows a signal; `on_change`
/// hears the whole new list after a reorder, a removal or an import, with no duplicates.
///
/// One gallery should be on screen at a time: Ctrl+V pastes into every gallery that is mounted.
pub fn gallery(
    label: impl Into<SharedString>,
    items: impl Fn() -> Vec<Attachment> + Copy + 'static,
    on_change: impl Fn(Vec<Attachment>) + Copy + 'static,
) -> impl IntoElement {
    let label: SharedString = label.into();
    let tile_label = label.clone();
    let folder = data_dir().map(|dir| dir.join("attachments"));
    let notice = signal(None::<String>);
    let columns = signal(1_usize);
    // Whoever hears a change (a save on a blocking thread) runs under the gallery's owner: the tile
    // that took a press is rebuilt as soon as the list changes, and a task of its owner would end
    // with it before the save is done.
    let owner = Owner::current();
    let on_change = move |list: Vec<Attachment>| owner.run(|| on_change(list));
    let removed = signal(None::<Removed>);
    let hooks = hooks(label.clone(), folder.as_deref(), items, on_change, removed);
    let undo = move || bring_back(removed, items, on_change);
    let intake: Rc<dyn Fn(Vec<PathBuf>)> = {
        let folder = folder.clone();
        Rc::new(move |paths: Vec<PathBuf>| {
            let (kept, note) = sift(paths, Accept::IMAGES, Pick::Many);
            notice.set(note);
            if !kept.is_empty() {
                store(folder.clone(), kept, items, on_change, notice);
            }
        })
    };
    let pasted = Rc::clone(&intake);
    on_key_with("mod+v", KeyOptions { pass_through: true, ..KeyOptions::default() }, move |_| {
        let pasted = Rc::clone(&pasted);
        spawn(async move {
            let images = pasted_images().await;
            if images.is_empty() {
                notice.set(Some(t!("ui.gallery.no_clipboard_image")));
            } else {
                pasted(images);
            }
        });
    });
    let dropped = Rc::clone(&intake);
    let picked = Rc::clone(&intake);
    view()
        .gap_3()
        .on_file_drop(move |event: &FileDropEvent| dropped(super::files::paths_of_uris(&event.uris)))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap(px(GAP))
                .on_layout(move |bounds: &Bounds<Points>| {
                    columns.set(grid_columns(bounds.size.width.0, TILE_WIDTH, GAP));
                })
                .child(dynamic(move || {
                    // Only the list decides the tiles: what building them reads is not tracked.
                    let list = items();
                    let len = list.len();
                    untracked(|| {
                        hooks.tiles.borrow_mut().clear();
                        list.into_iter()
                            .enumerate()
                            .map(|(index, item)| {
                                let place = Tile {
                                    label: tile_label.clone(),
                                    index,
                                    len,
                                    file: folder.as_deref().map(|dir| item.file_in(dir)),
                                    caption: caption_of(&item),
                                };
                                tile(place, hooks.clone(), items, on_change, columns)
                            })
                            .collect::<Vec<_>>()
                    })
                }))
                .child(add_tile(&label, move || {
                    let picked = Rc::clone(&picked);
                    spawn(async move {
                        match pick_files(Accept::IMAGES, Pick::Many).await {
                            Ok(paths) => picked(paths),
                            Err(error) => notice.set(Some(error)),
                        }
                    });
                })),
        )
        .child(dynamic(move || notice.get().map(error_note)))
        .child(dynamic(move || {
            // The line is there while the list is the one the picture left.
            let offered = removed.with(|gone| gone.as_ref().is_some_and(|gone| items() == gone.after));
            untracked(|| {
                offered.then(|| {
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(text(t!("ui.gallery.removed")).text_xs().text_color(move || theme().muted_foreground))
                        .child(link(t!("ui.gallery.undo"), undo))
                })
            })
        }))
}

/// A picture taken out of the list, and the list it left, to put it back.
#[derive(Clone, PartialEq)]
struct Removed {
    item: Attachment,
    index: usize,
    /// The list right after the removal.
    after: Vec<Attachment>,
}

/// What a tile calls into the gallery with.
#[derive(Clone)]
struct Hooks {
    /// Opens the viewer on a picture.
    view: Rc<dyn Fn(usize)>,
    /// Removes the picture at a position.
    remove: Rc<dyn Fn(usize)>,
    /// Where each tile's button is recorded, in order.
    tiles: Rc<RefCell<Vec<ElementRef>>>,
}

/// The tiles' hooks. Opening the viewer hands it the gallery's pictures to read live and a way
/// back to the tile it ends on; a removal takes the picture out of the list at once and keeps it
/// in `removed` for [`UNDO_WINDOW`]. Call while building the gallery: the timer and the viewer
/// belong to its owner.
fn hooks(
    label: SharedString,
    folder: Option<&std::path::Path>,
    items: impl Fn() -> Vec<Attachment> + Copy + 'static,
    on_change: impl Fn(Vec<Attachment>) + Copy + 'static,
    removed: Signal<Option<Removed>>,
) -> Hooks {
    let owner = Owner::current();
    let serial = signal(0_u32);
    let tiles: Rc<RefCell<Vec<ElementRef>>> = Rc::default();
    let slides: Slides = {
        let folder = folder.map(std::path::Path::to_path_buf);
        Rc::new(move || items().iter().map(|item| slide_of(item, folder.as_deref())).collect())
    };
    // A viewer that reads a gallery which is gone has nothing to read.
    cleanup({
        let slides = Rc::clone(&slides);
        move || viewer::close_if_from(&slides)
    });
    let restore: Rc<dyn Fn(usize)> = {
        let tiles = Rc::clone(&tiles);
        Rc::new(move |index| {
            if let Some(tile) = tiles.borrow().get(index) {
                tile.focus();
            }
        })
    };
    let view: Rc<dyn Fn(usize)> = Rc::new(move |index| {
        viewer::open(Request::new(label.clone(), Rc::clone(&slides), index, Rc::clone(&restore)));
    });
    let remove: Rc<dyn Fn(usize)> = Rc::new(move |index| {
        let mut list = items();
        if index >= list.len() {
            return;
        }
        let item = list.remove(index);
        removed.set(Some(Removed { item, index, after: list.clone() }));
        on_change(list);
        let this = serial.get_untracked().wrapping_add(1);
        serial.set(this);
        // Under the gallery's owner: the tile that took the press is gone with the list's change.
        owner.run(|| {
            set_timeout(UNDO_WINDOW, move || {
                if serial.get_untracked() == this {
                    removed.set(None);
                }
            })
        });
    });
    Hooks { view, remove, tiles }
}

/// Puts the removed picture back where it was, if the list is still the one it left: another
/// edit, or another entry in the gallery, ends the chance.
fn bring_back(
    removed: Signal<Option<Removed>>,
    items: impl Fn() -> Vec<Attachment>,
    on_change: impl Fn(Vec<Attachment>),
) {
    let Some(gone) = removed.get_untracked() else { return };
    removed.set(None);
    let mut list = items();
    if list == gone.after {
        list.insert(gone.index.min(list.len()), gone.item);
        on_change(list);
    }
}

/// What the viewer shows of a picture.
fn slide_of(item: &Attachment, folder: Option<&std::path::Path>) -> Slide {
    Slide { file: folder.map(|dir| item.file_in(dir)), caption: caption_of(item), detail: detail_of(item) }
}

/// When a picture was added, and how a picture of the old EA was taken.
fn detail_of(item: &Attachment) -> String {
    let added = (item.added_at > 0)
        .then(|| chrono::DateTime::from_timestamp_millis(item.added_at))
        .flatten()
        .map(|time| t!("ui.gallery.added_at", time = time.with_timezone(&chrono::Local).format("%d/%m/%Y %H:%M")));
    let taken = match item.source {
        Source::Upload => None,
        Source::EaEntry => Some(t!("ui.gallery.taken_at_entry")),
        Source::EaExit => Some(t!("ui.gallery.taken_at_exit")),
    };
    [added, taken].into_iter().flatten().collect::<Vec<_>>().join(" · ")
}

/// Copies `files` into the data folder, then reports the list with the new pictures added.
/// What could not be kept is said in `notice`.
fn store(
    folder: Option<PathBuf>,
    files: Vec<PathBuf>,
    items: impl Fn() -> Vec<Attachment> + 'static,
    on_change: impl Fn(Vec<Attachment>) + 'static,
    notice: Signal<Option<String>>,
) {
    let Some(folder) = folder else {
        notice.set(Some(t!("ui.gallery.no_folder")));
        return;
    };
    spawn(async move {
        let now = chrono::Utc::now().timestamp_millis();
        match blocking(move || attachments::import(&files, &folder, now)).await {
            Ok(results) => {
                let (kept, problems) = digest(results);
                if !kept.is_empty() {
                    let mut list = items();
                    attachments::append(&mut list, kept);
                    on_change(list);
                }
                // The note about refused files, set before the import, stays unless this one is worse.
                if problems.is_some() {
                    notice.set(problems);
                }
            }
            Err(error) => notice.set(Some(t!("ui.gallery.store_stopped", error = error))),
        }
    });
}

/// The pictures kept out of an import, and one line saying why the others were not.
fn digest(results: Vec<Result<Attachment, AttachError>>) -> (Vec<Attachment>, Option<String>) {
    let mut kept = Vec::new();
    let mut problems = Vec::new();
    for result in results {
        match result {
            Ok(item) => kept.push(item),
            Err(error) => problems.push(error.to_string()),
        }
    }
    (kept, (!problems.is_empty()).then(|| problems.join("; ")))
}

/// What a tile shows of its picture and where it stands in the list.
struct Tile {
    label: SharedString,
    index: usize,
    len: usize,
    /// The picture's file in the data folder's `attachments/`.
    file: Option<PathBuf>,
    caption: String,
}

/// The line under a picture: its timeframe (`h4` shows as `H4`) and caption.
pub fn caption_of(item: &Attachment) -> String {
    let timeframe = item.timeframe.as_deref().map(str::to_uppercase).filter(|t| !t.is_empty());
    match (timeframe, item.caption.trim()) {
        (Some(timeframe), "") => timeframe,
        (Some(timeframe), caption) => format!("{timeframe} · {caption}"),
        (None, caption) => caption.to_owned(),
    }
}

/// The list with the picture at `from` moved to `to`.
fn moved(mut list: Vec<Attachment>, from: usize, to: usize) -> Vec<Attachment> {
    attachments::reorder(&mut list, from, to);
    list
}

fn tile(
    place: Tile,
    hooks: Hooks,
    items: impl Fn() -> Vec<Attachment> + Copy + 'static,
    on_change: impl Fn(Vec<Attachment>) + Copy + 'static,
    columns: Signal<usize>,
) -> impl IntoElement {
    let Tile { label, index, len, file, caption } = place;
    let Hooks { view: view_picture, remove, tiles } = hooks;
    let name = format!("{label} {}", index + 1);
    let button_ref = element_ref();
    tiles.borrow_mut().push(button_ref);
    let reorder = move |to: usize| {
        if to != index {
            on_change(moved(items(), index, to));
        }
    };
    let by_drag = move |dx: f32, dy: f32| {
        let cell = (TILE_WIDTH + GAP, TILE_IMAGE + TILE_CAPTION + GAP);
        reorder(grid_drop_index(index, (dx, dy), cell, columns.get_untracked(), len));
    };
    let by_key = move |step: Step| reorder(stepped(index, step, len));
    let open = {
        let view_picture = Rc::clone(&view_picture);
        move || view_picture(index)
    };
    let open_too = move || view_picture(index);
    let take_out = move || remove(index);
    // The actions show over the tile while the pointer is on it or the focus is inside it.
    let actions = view()
        .flex_row()
        .items_center()
        .gap_1()
        .opacity(0.0_f32)
        .group_hover(|s| s.opacity(1.0))
        .group_focus_within(|s| s.opacity(1.0))
        .child(chip_button(Icon::Eye, Some(t!("ui.gallery.view")), t!("ui.gallery.view_tile", name = name), open_too))
        .child(chip_button(Icon::Close, None, t!("ui.gallery.remove", name = name), take_out));
    view()
        .w(px(TILE_WIDTH))
        .group()
        .child(
            view()
                .h(px(TILE_IMAGE))
                .child(
                    button()
                        .element_ref(&button_ref)
                        .cursor_pointer()
                        .on_press(open)
                        .accessibility_label(t!("ui.gallery.zoom", name = name))
                        .w_full()
                        .h(px(TILE_IMAGE))
                        .rounded(move || theme().radius)
                        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                        .child(image_slot(name.clone(), move || file.clone(), TILE_IMAGE).w_full()),
                )
                .child(
                    view()
                        .absolute()
                        .top(px(6.0))
                        .right(px(6.0))
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .child(actions)
                        .child(drag_handle(t!("ui.reorder.drag", name = name), by_drag, by_key)),
                ),
        )
        .child(view().justify_center().h(px(TILE_CAPTION)).child(
            text(caption).min_w(px(0.0)).number_of_lines(1).text_xs().text_color(move || theme().muted_foreground),
        ))
}

/// A small button over a tile's corner: an icon, and a word after it when `word` is given.
/// `label` is what screen readers say and tests press.
fn chip_button(glyph: Icon, word: Option<String>, label: String, on_press: impl Fn() + 'static) -> impl IntoElement {
    button()
        .cursor_pointer()
        .on_press(on_press)
        .accessibility_label(label)
        .h(px(CHIP))
        .min_w(px(CHIP))
        .flex_row()
        .items_center()
        .justify_center()
        .gap_1()
        .px_1()
        .rounded(px(6.0))
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().popover)
        .hover(|s| s.bg(move || theme().accent))
        .active(|s| s.scale(motion::PRESS_SCALE))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .child(icon(glyph, 14.0).text_color(move || theme().foreground))
        .child(word.map(|word| text(word).text_xs().font_weight(FontWeight::MEDIUM)))
}

/// The tile that adds pictures: press it for the file dialog, or drop files on the gallery.
fn add_tile(label: &SharedString, on_press: impl Fn() + 'static) -> impl IntoElement {
    button()
        .cursor_pointer()
        .on_press(on_press)
        .accessibility_label(t!("ui.gallery.add_to", label = label))
        .w(px(TILE_WIDTH))
        .h(px(TILE_IMAGE))
        .items_center()
        .justify_center()
        .gap_1()
        .rounded(move || theme().radius)
        .border_2()
        .border_dashed()
        .border_color(move || theme().border)
        .hover(|s| s.bg(move || theme().accent))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .child(icon(Icon::Plus, 20.0).text_color(move || theme().muted_foreground))
        .child(text(t!("ui.gallery.add")).text_xs().font_weight(FontWeight::MEDIUM))
        .child(text(t!("ui.gallery.add_hint")).text_xs().text_color(move || theme().muted_foreground))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nsh_core::{Size, platform::Os};
    use nsh_engine::{Clock, EngineConfig};
    use nsh_testing::{DragOptions, Headless};
    use tradelens_core::attachments::Source;

    use super::*;

    fn shot(path: &str, source: Source, caption: &str, timeframe: Option<&str>) -> Attachment {
        Attachment {
            path: path.to_owned(),
            caption: caption.to_owned(),
            timeframe: timeframe.map(str::to_owned),
            source,
            added_at: 0,
        }
    }

    #[test]
    fn a_caption_joins_the_timeframe_and_the_note() {
        assert_eq!(caption_of(&shot("a", Source::Upload, "bối cảnh trend", Some("h4"))), "H4 · bối cảnh trend");
        assert_eq!(caption_of(&shot("a", Source::EaEntry, "", Some("m15"))), "M15");
        assert_eq!(caption_of(&shot("a", Source::Upload, " retest ", None)), "retest");
        assert_eq!(caption_of(&shot("a", Source::Upload, "", None)), "");
    }

    #[test]
    fn an_import_keeps_the_pictures_and_says_why_one_was_refused() {
        let kept = shot("images/a.png", Source::Upload, "", None);
        let refused = AttachError::NotAnImage { name: "notes.txt".to_owned() };
        let (list, problems) = digest(vec![Ok(kept.clone()), Err(refused)]);
        assert_eq!(list, [kept]);
        assert!(problems.is_some_and(|p| p.contains("notes.txt")));
        assert_eq!(digest(Vec::new()), (Vec::new(), None));
    }

    #[test]
    fn moving_a_picture_keeps_the_others_in_order() {
        let list = vec![
            shot("a", Source::Upload, "", None),
            shot("b", Source::Upload, "", None),
            shot("c", Source::Upload, "", None),
        ];
        let paths = |l: &[Attachment]| l.iter().map(|a| a.path.clone()).collect::<Vec<_>>().join(",");
        assert_eq!(paths(&moved(list.clone(), 0, 2)), "b,c,a");
        assert_eq!(paths(&moved(list.clone(), 2, 0)), "c,a,b");
        assert_eq!(paths(&moved(list, 7, 0)), "a,b,c", "an unknown picture changes nothing");
    }

    #[component]
    fn Harness() -> impl IntoElement {
        super::super::theme::install();
        crate::i18n::install();
        let list = signal(vec![
            shot("images/a.png", Source::EaEntry, "bối cảnh", Some("h4")),
            shot("images/b.png", Source::Upload, "retest", Some("m5")),
            shot("images/c.png", Source::EaExit, "", None),
        ]);
        // Like the window: the page, then the viewer over it as the last child.
        crate::i18n::translated(move || {
            view()
                .flex_1()
                .child(
                    view()
                        .p_4()
                        .gap_3()
                        .child(gallery("Ảnh chart", move || list.get(), move |next| list.set(next)))
                        .child(text(move || {
                            format!(
                                "order {}",
                                list.with(|l| l.iter().map(|a| a.path.as_str()).collect::<Vec<_>>().join(","))
                            )
                        })),
                )
                .child(super::super::viewer::overlay())
        })
    }

    fn fixture() -> (tempfile::TempDir, Headless) {
        let dir = tempfile::tempdir().expect("a temp dir");
        let mut config = EngineConfig {
            viewport: Size::new(px(1000.0), px(700.0)),
            data_dir: Some(dir.path().to_path_buf()),
            ..EngineConfig::new(nsh_testing::windowed_root::<Harness>())
        };
        // A desktop that delivers keys to the focused view, and takes dropped files.
        config.environment.os = Os::Linux;
        let mut app = Headless::start(config).expect("the engine starts");
        app.wait_for("order images/a.png,images/b.png,images/c.png").expect("the harness shows");
        (dir, app)
    }

    #[test]
    fn captions_are_shown_and_no_picture_carries_a_badge() {
        let (_dir, app) = fixture();
        let badges: Vec<&str> = app.screen().texts().into_iter().filter(|t| t.contains("Tự chụp")).collect();
        assert!(badges.is_empty(), "the EA is gone, so is what said a picture came from it: {badges:?}");
        assert!(app.screen().shows("H4 · bối cảnh"));
        assert!(app.screen().shows("Thêm ảnh"));
    }

    #[test]
    fn dragging_a_handle_reports_the_new_order() {
        let (_dir, mut app) = fixture();
        // One tile and a gap to the right: the first picture lands second.
        app.drag("label:Kéo để đổi thứ tự Ảnh chart 1", point(px(210.0), px(0.0)), DragOptions::default())
            .expect("drags");
        app.wait_for("order images/b.png,images/a.png,images/c.png").expect("the callback got the new order");
    }

    #[test]
    fn the_arrow_keys_move_a_focused_handle() {
        let (_dir, mut app) = fixture();
        // Tab visits each tile's button, "Xem", remove button and handle in turn: the second
        // handle is the eighth stop.
        app.press_keys(&["tab"; 8]).expect("Tab moves the focus");
        app.press_key("right").expect("a key press");
        app.wait_for("order images/a.png,images/c.png,images/b.png").expect("one place later");
    }

    #[test]
    fn a_picture_is_removed_by_its_button_and_comes_back_by_undo() {
        let (_dir, mut app) = fixture();
        assert!(!app.screen().shows("Hoàn tác"), "nothing to undo yet");
        app.press("label:Xoá Ảnh chart 2").expect("the remove button");
        app.wait_for("order images/a.png,images/c.png").expect("the list lost it at once");
        app.wait_for("Đã xoá ảnh ·").expect("and says so");
        app.press("role:button[Hoàn tác]").expect("the undo link");
        app.wait_for("order images/a.png,images/b.png,images/c.png").expect("the picture is back in its place");
        app.wait_for_gone("Đã xoá ảnh ·", Duration::from_secs(3)).expect("the line goes with it");
    }

    #[test]
    fn another_edit_ends_the_chance_to_undo_the_first() {
        let (_dir, mut app) = fixture();
        app.press("label:Xoá Ảnh chart 1").expect("the remove button");
        app.wait_for("Đã xoá ảnh ·").expect("the line is there");
        app.press("label:Xoá Ảnh chart 1").expect("another picture goes");
        app.wait_for("order images/c.png").expect("the list is shorter again");
        app.wait_for("Đã xoá ảnh ·").expect("the line now offers the last picture");
        app.press("role:button[Hoàn tác]").expect("the undo link");
        app.wait_for("order images/b.png,images/c.png").expect("the last removal is undone, not the first");
    }

    #[test]
    fn the_undo_line_goes_after_a_few_seconds() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let mut config = EngineConfig {
            viewport: Size::new(px(1000.0), px(700.0)),
            data_dir: Some(dir.path().to_path_buf()),
            clock: Clock::manual(std::time::SystemTime::UNIX_EPOCH),
            ..EngineConfig::new(nsh_testing::windowed_root::<Harness>())
        };
        config.environment.os = Os::Linux;
        let mut app = Headless::start(config).expect("the engine starts");
        app.wait_for("order images/a.png,images/b.png,images/c.png").expect("the harness shows");
        app.press("label:Xoá Ảnh chart 3").expect("the remove button");
        app.wait_for("Đã xoá ảnh ·").expect("the line is there");
        app.fast_forward(Duration::from_secs(3)).expect("time passes");
        assert!(app.screen().shows("Đã xoá ảnh ·"), "still there after 3 s");
        app.fast_forward(Duration::from_secs(4)).expect("more time passes");
        app.wait_for_gone("Đã xoá ảnh ·", Duration::from_secs(3)).expect("gone after 7 s");
    }

    #[test]
    fn pressing_a_tile_opens_the_viewer_on_that_picture() {
        let (_dir, mut app) = fixture();
        assert!(!app.screen().shows("1 / 3"), "closed to begin with");
        app.press("label:Phóng to Ảnh chart 2").expect("the tile");
        app.wait_for("2 / 3").expect("the counter says which picture");
        assert_eq!(app.screen().count("text:M5 · retest").expect("counts"), 2, "the caption under the picture too");
        assert!(app.screen().shows("← → chuyển ảnh   + − phóng to / thu nhỏ   0 vừa khung   Esc đóng"));
    }

    #[test]
    fn the_view_button_of_a_tile_opens_it_too() {
        let (_dir, mut app) = fixture();
        app.press("label:Xem Ảnh chart 3").expect("the Xem button");
        app.wait_for("3 / 3").expect("the third picture");
    }

    #[test]
    fn the_arrows_move_between_pictures_and_wrap() {
        let (_dir, mut app) = fixture();
        app.press("label:Phóng to Ảnh chart 2").expect("the tile");
        app.wait_for("2 / 3").expect("opened");
        app.press_key("right").expect("next");
        app.wait_for("3 / 3").expect("the third");
        app.press_key("right").expect("next past the end");
        app.wait_for("1 / 3").expect("wraps to the first");
        app.press_key("left").expect("previous before the start");
        app.wait_for("3 / 3").expect("wraps to the last");
        app.press("label:Ảnh trước").expect("the previous button");
        app.wait_for("2 / 3").expect("the second");
        app.press("label:Ảnh sau").expect("the next button");
        app.wait_for("3 / 3").expect("the third again");
    }

    #[test]
    fn escape_closes_the_viewer_and_the_focus_returns_to_the_tile() {
        let (_dir, mut app) = fixture();
        app.press("label:Phóng to Ảnh chart 2").expect("the tile");
        app.wait_for("2 / 3").expect("opened");
        app.press_key("escape").expect("Escape");
        app.wait_for_gone("2 / 3", Duration::from_secs(3)).expect("closed");
        // Enter presses the focused control: it is the tile that opened the viewer.
        app.press_key("enter").expect("Enter");
        app.wait_for("2 / 3").expect("the tile had the focus, so Enter opened the same picture again");
    }

    #[test]
    fn the_focus_returns_to_the_tile_of_the_picture_last_shown() {
        let (_dir, mut app) = fixture();
        app.press("label:Phóng to Ảnh chart 1").expect("the tile");
        app.wait_for("1 / 3").expect("opened");
        app.press_key("right").expect("next");
        app.wait_for("2 / 3").expect("the second");
        app.press_key("escape").expect("Escape");
        app.wait_for_gone("2 / 3", Duration::from_secs(3)).expect("closed");
        app.press_key("enter").expect("Enter");
        app.wait_for("2 / 3").expect("the focus went to the second tile");
    }

    #[test]
    fn the_close_button_and_the_scrim_close_the_viewer() {
        let (_dir, mut app) = fixture();
        app.press("label:Phóng to Ảnh chart 1").expect("the tile");
        app.wait_for("1 / 3").expect("opened");
        app.press("label:Đóng ảnh").expect("the close button");
        app.wait_for_gone("1 / 3", Duration::from_secs(3)).expect("closed by the button");

        app.press("label:Phóng to Ảnh chart 1").expect("the tile");
        app.wait_for("1 / 3").expect("opened again");
        // The left edge of the window is outside the picture and the controls.
        app.press_at("label:Đóng trình xem ảnh", point(px(4.0), px(300.0))).expect("the scrim");
        app.wait_for_gone("1 / 3", Duration::from_secs(3)).expect("closed by the scrim");
    }

    #[test]
    fn the_zoom_toggle_and_keys_change_the_scale() {
        let (_dir, mut app) = fixture();
        app.press("label:Phóng to Ảnh chart 1").expect("the tile");
        app.wait_for("1 / 3").expect("opened");
        assert!(app.screen().shows("100%"), "fitted to the window");
        app.press_key("=").expect("+");
        app.wait_for("125%").expect("one step in");
        app.press_key("0").expect("0");
        app.wait_for("role:button[100%]").expect("fit");
        app.press("role:button[100%]").expect("the toggle");
        app.wait_for("200%").expect("a plain magnification, the real size is not known here");
        app.wait_for("role:button[Vừa khung]").expect("the toggle now says how to go back");
        app.press("role:button[Vừa khung]").expect("the toggle");
        app.wait_for("role:button[100%]").expect("fitted again");

        app.press_key("=").expect("+ after a press on a button");
        app.wait_for("125%").expect("one step in");
        app.press_key("-").expect("-");
        app.press_key("-").expect("- again, which stops at fit");
        app.wait_for("role:button[100%]").expect("never smaller than the window");
        app.press("label:Phóng to").expect("the zoom in button");
        app.wait_for("125%").expect("one step in by the button");
        app.press_key("0").expect("0");
        app.wait_for("role:button[100%]").expect("fit");
    }

    #[test]
    fn the_wheel_zooms() {
        let (_dir, mut app) = fixture();
        app.press("label:Phóng to Ảnh chart 1").expect("the tile");
        app.wait_for("1 / 3").expect("opened");
        app.wheel("label:Đóng trình xem ảnh", point(px(0.0), px(-120.0))).expect("the wheel pushed away");
        app.wait_for("125%").expect("zoomed in");
        app.wheel("label:Đóng trình xem ảnh", point(px(0.0), px(120.0))).expect("the wheel pulled back");
        app.wait_for("role:button[100%]").expect("zoomed out");
    }

    #[test]
    fn the_keyboard_reaches_a_tile_and_opens_it() {
        let (_dir, mut app) = fixture();
        app.press_key("tab").expect("Tab goes to the first tile");
        app.press_key("enter").expect("Enter presses it");
        app.wait_for("1 / 3").expect("the viewer is open on the first picture");
    }

    #[test]
    fn a_picture_can_be_opened_with_another_program_and_shown_in_its_folder() {
        let (dir, mut app) = fixture();
        app.on_pick_paths(Vec::new()).expect("the platform has a file manager");
        app.press("label:Phóng to Ảnh chart 2").expect("the tile");
        app.wait_for("2 / 3").expect("opened");
        app.press("role:button[Mở bằng ứng dụng khác]").expect("open with another program");
        app.press("role:button[Hiện trong thư mục]").expect("show in the folder");
        // Compared as strings: `Path`'s `==` reads `\` and `/` alike on Windows, where the shell
        // refuses to show a path that mixes them.
        let wanted = dir.path().join("attachments").join("images").join("b.png");
        let wanted = wanted.as_os_str();
        let requests = app.screen().file_requests().to_vec();
        assert!(
            requests
                .iter()
                .any(|r| matches!(r, nsh_core::file_dialog::FileRequest::OpenPath(p) if p.as_os_str() == wanted)),
            "{requests:?}"
        );
        assert!(
            requests
                .iter()
                .any(|r| matches!(r, nsh_core::file_dialog::FileRequest::RevealPath(p) if p.as_os_str() == wanted)),
            "{requests:?}"
        );
    }

    #[test]
    fn the_viewer_says_when_the_picture_was_added() {
        crate::i18n::with_text(crate::i18n::Lang::Vi, || {
            let added = Attachment { added_at: 1_700_000_000_000, ..shot("a", Source::EaEntry, "", None) };
            let detail = detail_of(&added);
            assert!(detail.starts_with("Thêm lúc "), "{detail}");
            assert!(detail.ends_with("· chụp lúc vào lệnh"), "{detail}");
            assert_eq!(detail_of(&shot("a", Source::Upload, "", None)), "", "an upload with no time says nothing");
        });
    }

    #[test]
    fn dropped_and_picked_files_are_stored_and_added() {
        let (dir, mut app) = fixture();
        let source = dir.path().join("h1.png");
        std::fs::write(&source, b"png one").expect("writes the source");
        let notes = dir.path().join("notes.txt");
        std::fs::write(&notes, b"text").expect("writes the text file");
        let uri = |p: &std::path::Path| url::Url::from_file_path(p).map(String::from).expect("absolute");

        app.drop_files("label:Thêm ảnh vào Ảnh chart", vec![uri(&source), uri(&notes)]).expect("drops");
        app.wait_for_within("text:/^order .*images\\/[0-9a-f]{16}\\.png$/", Duration::from_secs(3))
            .expect("the dropped picture was copied and added");
        assert!(app.screen().shows("1 file không đúng loại (Ảnh)"), "the text file is reported");
        assert!(dir.path().join("attachments").join("images").is_dir(), "copied into the data folder");

        // The same picture picked again is stored once and not listed twice.
        app.on_pick_paths(vec![source]).expect("answers the dialog");
        app.press("label:Thêm ảnh vào Ảnh chart").expect("the add tile");
        app.wait_for_gone("1 file không đúng loại (Ảnh)", Duration::from_secs(3))
            .expect("a clean pick clears the note");
        assert!(
            app.screen().texts().iter().any(|t| t.starts_with("order ") && t.matches(".png").count() == 4),
            "{:?}",
            app.screen().texts()
        );
    }
}
