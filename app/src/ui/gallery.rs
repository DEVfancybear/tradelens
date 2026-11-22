//! The picture gallery of an entry (a trade, a missed trade, a setup): a wrapping grid of
//! thumbnails with the EA's entry / exit badges, a caption, a handle to reorder (drag, or the
//! arrow keys) and a button to remove, ending in the "Thêm ảnh" tile that takes several files
//! from the file dialog, from a drop on the gallery, or from Ctrl+V.
//!
//! The gallery edits a list of [`Attachment`]s it does not own: every change is reported whole
//! through `on_change`, and saving the list is the caller's job. New files are copied into the
//! data folder with `tradelens_core::attachments::import` on a blocking thread first.

use std::{path::PathBuf, rc::Rc};

use nsh_base::{button, theme};
use nsh_core::prelude::*;
use tradelens_core::attachments::{self, AttachError, Attachment, Source};

use super::{
    chips::{Tone, chip},
    controls::{Variant, icon_action},
    files::{Accept, Pick, image_slot, pasted_images, pick_files, sift},
    icons::{Icon, icon},
    reorder::{Step, drag_handle, grid_columns, grid_drop_index, stepped},
    theme as tokens,
};

/// Width of a tile, in points.
const TILE_WIDTH: f32 = 196.0;
/// Height of a tile's picture.
const TILE_IMAGE: f32 = 120.0;
/// Room under the picture for the caption row.
const TILE_CAPTION: f32 = 42.0;
/// Space between tiles.
const GAP: f32 = 12.0;

/// The gallery of `items`, named `label` (`"Ảnh chart"`) for screen readers, whose tiles read
/// "label 1", "label 2". `items` is read where it is bound, so it follows a signal; `on_change`
/// hears the whole new list after a reorder, a removal or an import, with no duplicates.
///
/// One gallery should be on screen at a time: Ctrl+V pastes into every gallery that is mounted.
pub fn gallery(
    label: &'static str,
    items: impl Fn() -> Vec<Attachment> + Copy + 'static,
    on_change: impl Fn(Vec<Attachment>) + Copy + 'static,
) -> impl IntoElement {
    let folder = data_dir().map(|dir| dir.join("attachments"));
    let notice = signal(None::<String>);
    let columns = signal(1_usize);
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
                notice.set(Some("Clipboard không có ảnh hay file ảnh.".to_owned()));
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
                        list.into_iter()
                            .enumerate()
                            .map(|(index, item)| {
                                let place = Tile {
                                    label,
                                    index,
                                    len,
                                    file: folder.as_ref().map(|dir| dir.join(&item.path)),
                                    badge: badge_of(item.source),
                                    caption: caption_of(&item),
                                };
                                tile(place, items, on_change, columns)
                            })
                            .collect::<Vec<_>>()
                    })
                }))
                .child(add_tile(label, move || {
                    let picked = Rc::clone(&picked);
                    spawn(async move {
                        match pick_files(Accept::IMAGES, Pick::Many).await {
                            Ok(paths) => picked(paths),
                            Err(error) => notice.set(Some(error)),
                        }
                    });
                })),
        )
        .child(dynamic(move || notice.get().map(|line| text(line).text_xs().text_color(tokens::loss()))))
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
        notice.set(Some("Chưa có thư mục dữ liệu để lưu ảnh.".to_owned()));
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
            Err(error) => notice.set(Some(format!("Luồng lưu ảnh bị dừng: {error}"))),
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
    label: &'static str,
    index: usize,
    len: usize,
    /// The picture's file in the data folder's `attachments/`.
    file: Option<PathBuf>,
    badge: Option<&'static str>,
    caption: String,
}

/// The badge a picture carries by where it came from: the EA's shots say so.
fn badge_of(source: Source) -> Option<&'static str> {
    match source {
        Source::Upload => None,
        Source::EaEntry => Some("EA · vào lệnh"),
        Source::EaExit => Some("EA · ra lệnh"),
    }
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
    items: impl Fn() -> Vec<Attachment> + Copy + 'static,
    on_change: impl Fn(Vec<Attachment>) + Copy + 'static,
    columns: Signal<usize>,
) -> impl IntoElement {
    let Tile { label, index, len, file, badge, caption } = place;
    let name = format!("{label} {}", index + 1);
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
    let remove = move || {
        let mut list = items();
        if index < list.len() {
            list.remove(index);
            on_change(list);
        }
    };
    view()
        .w(px(TILE_WIDTH))
        .child(
            image_slot(name.clone(), move || file.clone(), TILE_IMAGE)
                .when_some(badge, |slot, badge| {
                    slot.child(view().absolute().top(px(8.0)).left(px(8.0)).child(chip(badge, Tone::Primary)))
                })
                .child(view().absolute().top(px(6.0)).right(px(6.0)).child(drag_handle(
                    format!("Kéo để đổi thứ tự {name}"),
                    by_drag,
                    by_key,
                ))),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1()
                .h(px(TILE_CAPTION))
                .child(
                    text(caption)
                        .flex_1()
                        .min_w(px(0.0))
                        .number_of_lines(1)
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                )
                .child(icon_action(Icon::Close, format!("Xoá {name}"), Variant::Ghost, remove)),
        )
}

/// The tile that adds pictures: press it for the file dialog, or drop files on the gallery.
fn add_tile(label: &'static str, on_press: impl Fn() + 'static) -> impl IntoElement {
    button()
        .on_press(on_press)
        .accessibility_label(format!("Thêm ảnh vào {label}"))
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
        .child(text("Thêm ảnh").text_xs().font_weight(FontWeight::MEDIUM))
        .child(text("chọn nhiều, kéo thả, Ctrl+V").text_xs().text_color(move || theme().muted_foreground))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nsh_core::{Size, platform::Os};
    use nsh_engine::{EngineConfig, Root};
    use nsh_testing::Headless;

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
    fn only_the_eas_shots_carry_a_badge() {
        assert_eq!(badge_of(Source::Upload), None);
        assert_eq!(badge_of(Source::EaEntry), Some("EA · vào lệnh"));
        assert_eq!(badge_of(Source::EaExit), Some("EA · ra lệnh"));
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
        let list = signal(vec![
            shot("images/a.png", Source::EaEntry, "bối cảnh", Some("h4")),
            shot("images/b.png", Source::Upload, "retest", Some("m5")),
            shot("images/c.png", Source::EaExit, "", None),
        ]);
        view().p_4().gap_3().child(gallery("Ảnh chart", move || list.get(), move |next| list.set(next))).child(text(
            move || format!("order {}", list.with(|l| l.iter().map(|a| a.path.as_str()).collect::<Vec<_>>().join(","))),
        ))
    }

    fn fixture() -> (tempfile::TempDir, Headless) {
        let dir = tempfile::tempdir().expect("a temp dir");
        let mut config = EngineConfig {
            viewport: Size::new(px(1000.0), px(700.0)),
            data_dir: Some(dir.path().to_path_buf()),
            ..EngineConfig::new(Root::of::<Harness>())
        };
        // A desktop that delivers keys to the focused view, and takes dropped files.
        config.environment.os = Os::Linux;
        let mut app = Headless::start(config).expect("the engine starts");
        app.wait_for("order images/a.png,images/b.png,images/c.png").expect("the harness shows");
        (dir, app)
    }

    #[test]
    fn the_ea_badges_and_captions_are_shown() {
        let (_dir, app) = fixture();
        assert!(app.screen().shows("EA · vào lệnh"));
        assert!(app.screen().shows("EA · ra lệnh"));
        assert!(app.screen().shows("H4 · bối cảnh"));
        assert!(app.screen().shows("Thêm ảnh"));
    }

    #[test]
    fn dragging_a_handle_reports_the_new_order() {
        let (_dir, mut app) = fixture();
        // One tile and a gap to the right: the first picture lands second.
        app.drag("label:Kéo để đổi thứ tự Ảnh chart 1", point(px(210.0), px(0.0))).expect("drags");
        app.wait_for("order images/b.png,images/a.png,images/c.png").expect("the callback got the new order");
    }

    #[test]
    fn the_arrow_keys_move_a_focused_handle() {
        let (_dir, mut app) = fixture();
        // Tab visits the handle and the remove button of each picture in turn: the second
        // handle is the third stop.
        app.press_keys(&["tab", "tab", "tab"]).expect("Tab moves the focus");
        app.press_key("right").expect("a key press");
        app.wait_for("order images/a.png,images/c.png,images/b.png").expect("one place later");
    }

    #[test]
    fn a_picture_is_removed_by_its_button() {
        let (_dir, mut app) = fixture();
        app.press("label:Xoá Ảnh chart 2").expect("the remove button");
        app.wait_for("order images/a.png,images/c.png").expect("the list lost it");
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
