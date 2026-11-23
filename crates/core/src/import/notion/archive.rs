//! A Notion export as the `.zip` Notion hands out ("Markdown & CSV"): the database's CSV and the
//! images its files column names. [`read_archive`] picks the CSV and plans the import as for a
//! bare CSV; [`NotionArchive::attach_images`] then reads the images the rows name out of the
//! archive and stores them as the trades' screenshots.
//!
//! The CSV is found by its extension and its `_all` suffix, never by its name: Notion does not
//! mark the names as UTF-8, so a Vietnamese title reads as garbage here.

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use zip::ZipArchive;

use crate::attachments::{self, MAX_IMAGE_BYTES};
use crate::import::ImportError;

use super::cells::percent_decode;
use super::{NotionPlan, infer};

/// The suffix of the CSV Notion writes next to the one that keeps the view's filters.
const ALL_SUFFIX: &str = "_all.csv";

/// The CSV in an export is never larger than this, in bytes.
const MAX_CSV_BYTES: u64 = 64 * 1024 * 1024;

/// How many images a plan found in an archive and how many it did not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ImageCounts {
    pub attached: usize,
    pub missing: usize,
}

/// An opened export archive: where it is and which entries are images.
#[derive(Debug, Clone)]
pub struct NotionArchive {
    path: PathBuf,
    /// Entry index of each image by its path in the archive, lowercase with `/` separators.
    by_path: HashMap<String, usize>,
    /// Entry indexes of the images that share a file name, by that name.
    by_name: HashMap<String, Vec<usize>>,
}

fn key_of(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches("./").trim_start_matches('/').to_lowercase()
}

fn file_name_of(key: &str) -> &str {
    key.rsplit('/').next().unwrap_or(key)
}

/// Whether the entry is the macOS resource-fork clutter some archivers add.
fn is_clutter(path: &str) -> bool {
    path.starts_with("__MACOSX/") || file_name_of(path).starts_with("._")
}

fn invalid(error: impl std::fmt::Display) -> ImportError {
    ImportError::Invalid(format!("file zip không đọc được: {error}"))
}

/// Reads the plan of the export archive at `path`: the CSV is `*_all.csv` when there is one,
/// else the only `.csv`.
///
/// # Errors
///
/// [`ImportError::Io`] when the file cannot be opened; [`ImportError::Invalid`] when it is not a
/// zip, has no CSV or several without an `_all` one, or the CSV is not a trading journal.
pub fn read_archive(path: &Path) -> Result<(NotionPlan, NotionArchive), ImportError> {
    let mut zip = ZipArchive::new(File::open(path)?).map_err(invalid)?;
    let mut csv: Vec<(usize, String)> = Vec::new();
    let mut by_path = HashMap::new();
    let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
    for index in 0..zip.len() {
        let entry = zip.by_index(index).map_err(invalid)?;
        let full = entry.name().map_err(invalid)?.into_owned();
        if !entry.is_file() || is_clutter(&full) {
            continue;
        }
        let name = key_of(&full);
        if std::path::Path::new(&name).extension().is_some_and(|e| e.eq_ignore_ascii_case("csv")) {
            csv.push((index, name));
        } else if attachments::IMAGE_EXTENSIONS.iter().any(|e| name.ends_with(&format!(".{e}"))) {
            by_name.entry(file_name_of(&name).to_owned()).or_default().push(index);
            by_path.insert(name, index);
        }
    }
    let picked = match csv.iter().find(|(_, name)| name.ends_with(ALL_SUFFIX)) {
        Some((index, _)) => *index,
        None => match csv.as_slice() {
            [(index, _)] => *index,
            [] => return Err(ImportError::Invalid("file zip không có file .csv nào của Notion".into())),
            many => {
                return Err(ImportError::Invalid(format!(
                    "file zip có {} file .csv mà không có file «…_all.csv»: giải nén và chọn đúng file",
                    many.len()
                )));
            }
        },
    };
    let mut bytes = Vec::new();
    let entry = zip.by_index(picked).map_err(invalid)?;
    if entry.size() > MAX_CSV_BYTES {
        return Err(ImportError::Invalid("file CSV trong zip quá lớn".into()));
    }
    entry.take(MAX_CSV_BYTES).read_to_end(&mut bytes)?;
    let plan = infer(&bytes)?;
    Ok((plan, NotionArchive { path: path.to_path_buf(), by_path, by_name }))
}

impl NotionArchive {
    /// How many images the archive holds.
    pub fn image_count(&self) -> usize {
        self.by_path.len()
    }

    /// The entry a files-cell part names: by its whole path, else by its file name when only one
    /// image has it.
    fn find(&self, part: &str) -> Option<usize> {
        let key = key_of(&percent_decode(part));
        self.by_path.get(&key).copied().or_else(|| match self.by_name.get(file_name_of(&key)).map(Vec::as_slice) {
            Some([only]) => Some(*only),
            _ => None,
        })
    }

    /// How many of the file names the plan's rows hold are images of the archive, and how many are not.
    pub fn count(&self, plan: &NotionPlan) -> ImageCounts {
        let mut counts = ImageCounts::default();
        for name in plan.rows.iter().flat_map(|row| &row.journal.screenshots) {
            if self.find(name).is_some() {
                counts.attached += 1;
            } else {
                counts.missing += 1;
            }
        }
        counts
    }

    /// Stores the images the plan's rows name as their screenshots, in the data folder's
    /// `attachments/` (`store`), the same image once however many rows hold it.
    ///
    /// # Errors
    ///
    /// [`ImportError::Invalid`] when the archive can no longer be read, [`ImportError::Io`] when
    /// the store cannot be written. An image that is not an image, or too large, is counted as missing.
    pub fn attach_images(&self, plan: &mut NotionPlan, store: &Path, now_ms: i64) -> Result<ImageCounts, ImportError> {
        let mut zip = ZipArchive::new(File::open(&self.path)?).map_err(invalid)?;
        let mut read = |name: &str| -> Option<Vec<u8>> {
            let index = self.find(name)?;
            let entry = zip.by_index(index).ok()?;
            if entry.size() > MAX_IMAGE_BYTES {
                return None;
            }
            let mut bytes = Vec::new();
            entry.take(MAX_IMAGE_BYTES).read_to_end(&mut bytes).ok()?;
            Some(bytes)
        };
        plan.attach_images(&mut read, store, now_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_compare_without_case_separators_or_a_leading_dot() {
        assert_eq!(key_of(".\\Folder\\Image 1.PNG"), "folder/image 1.png");
        assert_eq!(file_name_of("a/b/c.png"), "c.png");
        assert!(is_clutter("__MACOSX/a.csv") && is_clutter("x/._a.png") && !is_clutter("x/a.png"));
    }
}
