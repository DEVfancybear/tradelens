//! Chart images attached to trades, missed trades and setups: several per entry, added
//! together (pick many files, drop many, paste), ordered, captioned and removed one by one.
//! Files are copied into the data folder under a name made from their content, so the
//! same image added twice is stored once and the originals can be moved or deleted.

use std::{
    fs,
    hash::Hasher,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

/// The largest image accepted, in bytes.
pub const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;

/// Extensions accepted, lowercase.
pub const IMAGE_EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "webp", "gif"];

/// Where an image came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// Picked, dropped or pasted by the trader.
    #[default]
    Upload,
    /// Taken by the EA when the position opened.
    EaEntry,
    /// Taken by the EA when the position closed.
    EaExit,
}

/// One image of an entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    /// Relative to the data folder's `attachments/`.
    pub path: String,
    #[serde(default)]
    pub caption: String,
    /// A key of the trader's timeframe catalog (`h1`, `m15`).
    #[serde(default)]
    pub timeframe: Option<String>,
    #[serde(default)]
    pub source: Source,
    /// Unix milliseconds.
    #[serde(default)]
    pub added_at: i64,
}

/// Why one file of a batch was not added; the others still are.
#[derive(Debug, thiserror::Error)]
pub enum AttachError {
    #[error("{name}: không phải ảnh (nhận PNG, JPG, WEBP, GIF)")]
    NotAnImage { name: String },
    #[error("{name}: ảnh lớn hơn 20 MB")]
    TooLarge { name: String },
    #[error("{name}: không đọc/ghi được ({source})")]
    Io { name: String, source: std::io::Error },
}

/// Copies `files` into `store` (the data folder's `attachments/`) and returns one result
/// per file, in order: an [`Attachment`] for each image kept, an error for each refused.
pub fn import(files: &[PathBuf], store: &Path, now_ms: i64) -> Vec<Result<Attachment, AttachError>> {
    files.iter().map(|file| import_one(file, store, now_ms)).collect()
}

fn import_one(file: &Path, store: &Path, now_ms: i64) -> Result<Attachment, AttachError> {
    let name = file.file_name().map_or_else(|| file.display().to_string(), |n| n.to_string_lossy().into_owned());
    let io = |source| AttachError::Io { name: name.clone(), source };
    let extension = file.extension().map_or_default(|e| e.to_string_lossy().to_ascii_lowercase());
    if !IMAGE_EXTENSIONS.contains(&extension.as_str()) {
        return Err(AttachError::NotAnImage { name });
    }
    let size = fs::metadata(file).map_err(io)?.len();
    if size > MAX_IMAGE_BYTES {
        return Err(AttachError::TooLarge { name });
    }
    let bytes = fs::read(file).map_err(io)?;
    import_bytes(&name, &bytes, store, now_ms)
}

/// Stores an image that is already in memory (one read out of an archive) under the name made from
/// its content, as [`import`] does for a file. `name` gives the extension and names the image in
/// errors.
///
/// # Errors
///
/// [`AttachError::NotAnImage`] for another extension, [`AttachError::TooLarge`] over
/// [`MAX_IMAGE_BYTES`], [`AttachError::Io`] when the store cannot be written.
pub fn import_bytes(name: &str, bytes: &[u8], store: &Path, now_ms: i64) -> Result<Attachment, AttachError> {
    let io = |source| AttachError::Io { name: name.to_owned(), source };
    let extension = Path::new(name).extension().map_or_default(|e| e.to_string_lossy().to_ascii_lowercase());
    if !IMAGE_EXTENSIONS.contains(&extension.as_str()) {
        return Err(AttachError::NotAnImage { name: name.to_owned() });
    }
    if u64::try_from(bytes.len()).is_ok_and(|size| size > MAX_IMAGE_BYTES) {
        return Err(AttachError::TooLarge { name: name.to_owned() });
    }
    let relative = format!("{}/{:016x}.{extension}", "images", content_hash(bytes));
    let target = store.join(&relative);
    if !target.exists() {
        if let Some(dir) = target.parent() {
            fs::create_dir_all(dir).map_err(io)?;
        }
        fs::write(&target, bytes).map_err(io)?;
    }
    Ok(Attachment { path: relative, caption: String::new(), timeframe: None, source: Source::Upload, added_at: now_ms })
}

/// FNV-1a over the bytes: a stable name for identical content (not a security hash).
fn content_hash(bytes: &[u8]) -> u64 {
    struct Fnv(u64);
    impl Hasher for Fnv {
        fn finish(&self) -> u64 {
            self.0
        }
        fn write(&mut self, bytes: &[u8]) {
            for b in bytes {
                self.0 ^= u64::from(*b);
                self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
            }
        }
    }
    let mut hasher = Fnv(0xcbf2_9ce4_8422_2325);
    hasher.write(bytes);
    hasher.finish()
}

/// Appends `added` to `list`, skipping images already in it.
pub fn append(list: &mut Vec<Attachment>, added: impl IntoIterator<Item = Attachment>) {
    for attachment in added {
        if !list.iter().any(|a| a.path == attachment.path) {
            list.push(attachment);
        }
    }
}

/// Moves the image at `from` to `to` (clamped), as a drag in the gallery does.
pub fn reorder(list: &mut Vec<Attachment>, from: usize, to: usize) {
    if from < list.len() {
        let item = list.remove(from);
        list.insert(to.min(list.len()), item);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn several_images_are_added_at_once_and_duplicates_kept_once() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let store = dir.path().join("attachments");
        let write = |name: &str, bytes: &[u8]| {
            let path = dir.path().join(name);
            fs::write(&path, bytes).expect("writes");
            path
        };
        let files = [
            write("h1.png", b"png one"),
            write("m15.JPG", b"jpg two"),
            write("notes.txt", b"text"),
            write("h1-copy.png", b"png one"),
        ];
        let results = import(&files, &store, 1_000);
        assert_eq!(results.len(), 4);
        assert!(matches!(results[2], Err(AttachError::NotAnImage { .. })));
        let kept: Vec<Attachment> = results.into_iter().filter_map(Result::ok).collect();
        assert_eq!(kept.len(), 3);
        assert_eq!(kept[0].path, kept[2].path, "same content, same file");
        assert!(store.join(&kept[1].path).exists());

        let mut gallery = Vec::new();
        append(&mut gallery, kept);
        assert_eq!(gallery.len(), 2);
        reorder(&mut gallery, 1, 0);
        assert_eq!(Path::new(&gallery[0].path).extension().and_then(|e| e.to_str()), Some("jpg"));
    }
}
