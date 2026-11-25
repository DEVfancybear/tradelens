//! Getting files into the app: the dashed drop zone, the picture slot, and the helpers that
//! turn what the platform hands over (a dialog's paths, dropped `file:` URLs, the clipboard)
//! into paths. Storing the files is the caller's job (`tradelens_core::attachments::import`
//! for pictures, `core::import` for reports).

use std::path::{Path, PathBuf};

use base64::{Engine, engine::general_purpose::STANDARD};
use nsh_base::{button, theme};
use nsh_core::{
    file_dialog::{FileFilter, OpenDialog},
    prelude::*,
};
use tradelens_core::attachments::IMAGE_EXTENSIONS;
use url::Url;

use super::icons::{Icon, icon};

/// Which files a drop zone or a picker takes: a name for the file dialog and the extensions
/// (lowercase, no dot).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Accept {
    pub name: &'static str,
    pub extensions: &'static [&'static str],
}

impl Accept {
    /// Chart images, as `tradelens_core::attachments` stores them.
    pub const IMAGES: Self = Self { name: "Ảnh", extensions: &IMAGE_EXTENSIONS };
    /// MT5 reports: HTML and XLSX.
    pub const REPORTS: Self = Self { name: "Báo cáo MT5", extensions: &["html", "htm", "xlsx"] };

    /// The filter the file dialog offers.
    fn filter(self) -> FileFilter {
        FileFilter::new(self.name, self.extensions.iter().copied())
    }

    /// Whether `path` has one of the extensions, in any case.
    pub fn allows(self, path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| self.extensions.iter().any(|x| x.eq_ignore_ascii_case(e)))
    }
}

/// How many files a pick or a drop may bring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    One,
    Many,
}

/// The local paths the `file:` URLs name; URLs that are not local files are skipped.
pub fn paths_of_uris(uris: &[String]) -> Vec<PathBuf> {
    uris.iter().filter_map(|uri| Url::parse(uri).ok()?.to_file_path().ok()).collect()
}

/// The `file:` URL of a local path, which an `image` shows; `None` for a relative path.
pub fn file_url(path: &Path) -> Option<String> {
    Url::from_file_path(path).ok().map(String::from)
}

/// Keeps the paths `accept` takes (the first one only for [`Pick::One`]) and says in a line
/// what was left out, if anything was.
pub fn sift(paths: Vec<PathBuf>, accept: Accept, pick: Pick) -> (Vec<PathBuf>, Option<String>) {
    let total = paths.len();
    let mut kept: Vec<PathBuf> = paths.into_iter().filter(|p| accept.allows(p)).collect();
    let skipped = total - kept.len();
    if pick == Pick::One {
        kept.truncate(1);
    }
    (kept, (skipped > 0).then(|| format!("{skipped} file không đúng loại ({})", accept.name)))
}

/// Shows the system's open dialog for `accept`. `Ok(vec![])` when the trader cancels; the error
/// (in Vietnamese) when the dialog could not open.
///
/// # Errors
///
/// A message for the trader when the platform has no file dialog or it failed to show.
pub async fn pick_files(accept: Accept, pick: Pick) -> Result<Vec<PathBuf>, String> {
    let dialog = OpenDialog::files().multiple(pick == Pick::Many).filter(accept.filter());
    match pick_paths(dialog).await {
        Ok(picked) => Ok(picked.unwrap_or_default()),
        Err(error) => Err(format!("Không mở được hộp thoại chọn file: {error}")),
    }
}

/// The pictures on the clipboard: files copied in the file manager, else an image copied from
/// a screenshot tool or a browser, written to a temporary file. Empty when there is none.
pub async fn pasted_images() -> Vec<PathBuf> {
    let copied = paths_of_uris(&clipboard_files().await);
    let images: Vec<PathBuf> = copied.into_iter().filter(|p| Accept::IMAGES.allows(p)).collect();
    if !images.is_empty() {
        return images;
    }
    let Some(image) = clipboard_image().await else { return Vec::new() };
    match blocking(move || write_pasted_image(&image.url)).await {
        Ok(Ok(path)) => vec![path],
        Ok(Err(error)) => {
            log::warn!("pasted image: {error}");
            Vec::new()
        }
        Err(error) => {
            log::warn!("pasted image: the writer stopped: {error}");
            Vec::new()
        }
    }
}

/// Why a pasted image could not be kept.
#[derive(Debug, thiserror::Error)]
enum PasteError {
    #[error("the clipboard image is not a base64 data URI")]
    NotAnImage,
    #[error("cannot write the temporary file: {0}")]
    Io(#[from] std::io::Error),
}

fn write_pasted_image(uri: &str) -> Result<PathBuf, PasteError> {
    let (extension, bytes) = decode_data_uri(uri).ok_or(PasteError::NotAnImage)?;
    let stamp = chrono::Utc::now().timestamp_millis();
    let path = std::env::temp_dir().join(format!("tradelens-paste-{stamp}.{extension}"));
    std::fs::write(&path, bytes)?;
    Ok(path)
}

/// The extension and bytes of a base64 `data:image/...` URI.
fn decode_data_uri(uri: &str) -> Option<(&'static str, Vec<u8>)> {
    let rest = uri.strip_prefix("data:image/")?;
    let (head, data) = rest.split_once(',')?;
    let mime = head.strip_suffix(";base64")?;
    let extension = match mime {
        "png" => "png",
        "jpeg" | "jpg" => "jpg",
        "webp" => "webp",
        "gif" => "gif",
        _ => return None,
    };
    STANDARD.decode(data.trim()).ok().map(|bytes| (extension, bytes))
}

/// A dashed box that takes files two ways: press it to open the file dialog, or drop files on
/// it. `on_files` hears the paths that pass `accept` (the dialog's and the drop's), at most one
/// when `pick` is [`Pick::One`]; files that do not pass are counted in a line inside the box.
/// Nothing is read or copied here.
pub fn dropzone(
    title: &'static str,
    hint: &'static str,
    accept: Accept,
    pick: Pick,
    on_files: impl Fn(Vec<PathBuf>) + Clone + 'static,
) -> impl IntoElement {
    let over = signal(false);
    let problem = signal(None::<String>);
    let deliver = move |paths: Vec<PathBuf>| {
        let (kept, note) = sift(paths, accept, pick);
        problem.set(note);
        if !kept.is_empty() {
            on_files(kept);
        }
    };
    let dropped = deliver.clone();
    let open = move || {
        let deliver = deliver.clone();
        spawn(async move {
            match pick_files(accept, pick).await {
                Ok(paths) => deliver(paths),
                Err(error) => problem.set(Some(error)),
            }
        });
    };
    button()
        .cursor_pointer()
        .on_press(open)
        .accessibility_label(title)
        .items_center()
        .justify_center()
        .gap_1()
        .py_6()
        .px_4()
        .rounded(move || theme().radius)
        .border_2()
        .border_dashed()
        .border_color(move || if over.get() { theme().primary } else { theme().border })
        .bg(move || if over.get() { theme().accent } else { AdaptiveColor::TRANSPARENT })
        .hover(|s| s.bg(move || theme().accent))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .on_file_drag_enter(move |_: &FileDragEvent| over.set(true))
        .on_file_drag_leave(move |(): &()| over.set(false))
        .on_file_drop(move |event: &FileDropEvent| {
            over.set(false);
            dropped(paths_of_uris(&event.uris));
        })
        .child(icon(Icon::Upload, 20.0).text_color(move || theme().muted_foreground))
        .child(text(title).text_sm().font_weight(FontWeight::MEDIUM))
        .child(text(hint).text_xs().text_color(move || theme().muted_foreground))
        .child(dynamic(move || problem.get().map(|line| text(line).text_xs().text_color(super::theme::loss))))
}

/// A picture of the app's data folder: a framed `height`-point box that shows the image at
/// `file` (an absolute path) cropped to fill it, over a placeholder that stays visible while
/// the image loads or when the file is gone. Returns the `View` so that badges and handles can
/// be added as children (they sit over the picture).
pub fn image_slot(
    description: impl Into<SharedString>,
    file: impl Fn() -> Option<PathBuf> + 'static,
    height: f32,
) -> View {
    let description: SharedString = description.into();
    view()
        .h(px(height))
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().secondary)
        .overflow_hidden()
        .items_center()
        .justify_center()
        .child(icon(Icon::Image, 24.0).text_color(move || theme().muted_foreground))
        .child(dynamic(move || {
            let description = description.clone();
            file()
                .and_then(|path| file_url(&path))
                .map(move |url| image(url).absolute().inset_0().object_fit(ResizeMode::Cover).label(description))
        }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_file_urls_become_paths_and_others_are_skipped() {
        let uris = vec![
            Url::from_file_path(std::env::temp_dir().join("a b.png")).map(String::from).expect("absolute"),
            "https://example.com/a.png".to_owned(),
            "not a url".to_owned(),
        ];
        let paths = paths_of_uris(&uris);
        assert_eq!(paths.len(), 1, "{paths:?}");
        assert_eq!(paths[0].file_name().and_then(|n| n.to_str()), Some("a b.png"), "spaces are decoded");
    }

    #[test]
    fn a_path_round_trips_through_its_url() {
        let path = std::env::temp_dir().join("chart h4.png");
        let url = file_url(&path).expect("an absolute path");
        assert_eq!(paths_of_uris(&[url]), vec![path]);
        assert_eq!(file_url(Path::new("relative.png")), None);
    }

    #[test]
    fn accept_checks_the_extension_in_any_case() {
        assert!(Accept::IMAGES.allows(Path::new("C:/shots/H1.PNG")));
        assert!(Accept::REPORTS.allows(Path::new("report.xlsx")));
        assert!(!Accept::IMAGES.allows(Path::new("notes.txt")));
        assert!(!Accept::IMAGES.allows(Path::new("noextension")));
    }

    #[test]
    fn sifting_keeps_what_is_accepted_and_counts_the_rest() {
        let files = vec![PathBuf::from("a.png"), PathBuf::from("b.txt"), PathBuf::from("c.jpg")];
        let (kept, note) = sift(files.clone(), Accept::IMAGES, Pick::Many);
        assert_eq!(kept, [PathBuf::from("a.png"), PathBuf::from("c.jpg")]);
        assert_eq!(note.as_deref(), Some("1 file không đúng loại (Ảnh)"));
        let (kept, note) = sift(files, Accept::IMAGES, Pick::One);
        assert_eq!(kept, [PathBuf::from("a.png")], "only the first for a single pick");
        assert!(note.is_some());
        assert_eq!(sift(vec![PathBuf::from("a.png")], Accept::IMAGES, Pick::Many).1, None);
    }

    #[test]
    fn a_clipboard_image_is_decoded_with_its_extension() {
        // "hi" in base64.
        let (extension, bytes) = decode_data_uri("data:image/png;base64,aGk=").expect("a png data uri");
        assert_eq!((extension, bytes.as_slice()), ("png", b"hi".as_slice()));
        assert_eq!(decode_data_uri("data:image/jpeg;base64,aGk=").map(|d| d.0), Some("jpg"));
        assert!(decode_data_uri("data:text/plain;base64,aGk=").is_none());
        assert!(decode_data_uri("data:image/png,raw").is_none());
        assert!(decode_data_uri("data:image/png;base64,***").is_none());
    }

    #[test]
    fn a_pasted_image_is_written_to_a_file() {
        let path = write_pasted_image("data:image/png;base64,aGk=").expect("writes");
        assert_eq!(std::fs::read(&path).expect("reads back"), b"hi");
        assert!(matches!(write_pasted_image("nope"), Err(PasteError::NotAnImage)));
        std::fs::remove_file(path).expect("cleans up");
    }
}
