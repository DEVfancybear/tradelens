//! Moving the data of an earlier version into the folder the app keeps it in now.
//!
//! Until the app had an id, NSH kept its data in a folder shared by every app without one
//! (`default`). [`adopt`] copies the database and the images from there to the app's own folder,
//! once, and leaves the old folder untouched.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use rusqlite::Connection;

/// The database's file name in a data folder.
pub const DB_FILE: &str = "tradelens.db";

/// The folder of the journal's images in a data folder.
pub const ATTACHMENTS_DIR: &str = "attachments";

/// The folder the copy is made in until it is complete.
const STAGING_DIR: &str = ".adopting";

/// What [`adopt`] copied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adopted {
    /// The folder the data came from.
    pub from: PathBuf,
    /// How many image files were copied.
    pub images: usize,
}

/// Why the data could not be copied.
#[derive(Debug, thiserror::Error)]
pub enum AdoptError {
    #[error("không chép được {what}: {source}")]
    Io {
        what: String,
        #[source]
        source: io::Error,
    },
    #[error("không gộp được nhật ký ghi dở của dữ liệu cũ: {0}")]
    Checkpoint(#[from] rusqlite::Error),
}

fn io_error(what: impl Into<String>) -> impl FnOnce(io::Error) -> AdoptError {
    let what = what.into();
    move |source| AdoptError::Io { what, source }
}

/// Copies the data of `legacy` into `target` when `target` has no database yet and `legacy` has one.
///
/// The database is copied with its write-ahead log and checkpointed on the copy, so what the old
/// folder's database had not folded in yet is in the new file; the images follow; the database
/// file is put in place last, so an interrupted copy leaves `target` without one and the next start
/// tries again. Nothing in `legacy` is written, moved or deleted.
///
/// Returns `None` when there was nothing to do: `target` already has a database, `legacy` has none,
/// or they are the same folder.
///
/// # Errors
///
/// [`AdoptError`] when a file cannot be copied or the copy of the database cannot be checkpointed.
pub fn adopt(target: &Path, legacy: &Path) -> Result<Option<Adopted>, AdoptError> {
    let source_db = legacy.join(DB_FILE);
    if target == legacy || target.join(DB_FILE).exists() || !source_db.is_file() {
        return Ok(None);
    }
    let staging = target.join(STAGING_DIR);
    if staging.exists() {
        fs::remove_dir_all(&staging).map_err(io_error("thư mục chép dở"))?;
    }
    fs::create_dir_all(&staging).map_err(io_error(format!("thư mục {}", staging.display())))?;

    // The database and its write-ahead log and shared memory, as a set.
    for suffix in ["", "-wal", "-shm"] {
        let name = format!("{DB_FILE}{suffix}");
        let from = legacy.join(&name);
        if !from.is_file() {
            continue;
        }
        let copied = fs::copy(&from, staging.join(&name));
        // The shared-memory file is rebuilt from the log when it is missing, and Windows may hold
        // it locked while the old app still runs; the other two are the data.
        if suffix != "-shm" {
            copied.map_err(io_error(from.display().to_string()))?;
        }
    }
    {
        let copy = Connection::open(staging.join(DB_FILE))?;
        copy.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
    }
    for suffix in ["-wal", "-shm"] {
        let leftover = staging.join(format!("{DB_FILE}{suffix}"));
        if leftover.exists() {
            fs::remove_file(&leftover).map_err(io_error(leftover.display().to_string()))?;
        }
    }

    let images = copy_missing(&legacy.join(ATTACHMENTS_DIR), &target.join(ATTACHMENTS_DIR))?;
    fs::rename(staging.join(DB_FILE), target.join(DB_FILE)).map_err(io_error("tệp dữ liệu"))?;
    // The staging folder is empty now; failing to remove it changes nothing.
    drop(fs::remove_dir_all(&staging));
    Ok(Some(Adopted { from: legacy.to_path_buf(), images }))
}

/// Copies the files of `from` that `to` lacks, folders included; returns how many files were copied.
fn copy_missing(from: &Path, to: &Path) -> Result<usize, AdoptError> {
    let Ok(entries) = fs::read_dir(from) else { return Ok(0) };
    fs::create_dir_all(to).map_err(io_error(format!("thư mục {}", to.display())))?;
    let mut copied = 0;
    for entry in entries {
        let entry = entry.map_err(io_error(from.display().to_string()))?;
        let (source, destination) = (entry.path(), to.join(entry.file_name()));
        if source.is_dir() {
            copied += copy_missing(&source, &destination)?;
        } else if !destination.exists() {
            fs::copy(&source, &destination).map_err(io_error(source.display().to_string()))?;
            copied += 1;
        }
    }
    Ok(copied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Db;

    /// A folder holding a database with a setting (still open, so its write-ahead log is unfolded), and an image.
    fn legacy_folder(root: &Path) -> (PathBuf, Db) {
        let legacy = root.join("default");
        let mut db = Db::open(legacy.join(DB_FILE)).expect("opens");
        db.set_setting("marker", "from the old folder").expect("writes");
        fs::create_dir_all(legacy.join(ATTACHMENTS_DIR).join("images")).expect("folder");
        fs::write(legacy.join(ATTACHMENTS_DIR).join("images").join("a.png"), b"png").expect("image");
        (legacy, db)
    }

    #[test]
    fn the_database_and_the_images_are_copied_and_the_old_folder_is_left_alone() {
        let root = tempfile::tempdir().expect("temp");
        let (legacy, _open) = legacy_folder(root.path());
        let target = root.path().join("vn.dat.tradelens");
        let before: Vec<_> = fs::read_dir(&legacy).expect("list").map(|e| e.expect("entry").file_name()).collect();

        let adopted = adopt(&target, &legacy).expect("copies").expect("there was something to copy");
        assert_eq!((adopted.from.as_path(), adopted.images), (legacy.as_path(), 1));

        let copy = Db::open(target.join(DB_FILE)).expect("the copy opens");
        assert_eq!(copy.setting("marker").expect("reads").as_deref(), Some("from the old folder"));
        assert_eq!(fs::read(target.join("attachments/images/a.png")).expect("image copied"), b"png");
        assert!(!target.join(STAGING_DIR).exists(), "the staging folder is gone");
        let after: Vec<_> = fs::read_dir(&legacy).expect("list").map(|e| e.expect("entry").file_name()).collect();
        assert_eq!(before, after, "nothing was added to or removed from the old folder");
        assert!(legacy.join(DB_FILE).exists() && legacy.join("attachments/images/a.png").exists());
    }

    #[test]
    fn a_target_that_has_a_database_is_never_touched() {
        let root = tempfile::tempdir().expect("temp");
        let (legacy, _open) = legacy_folder(root.path());
        let target = root.path().join("vn.dat.tradelens");
        let mut own = Db::open(target.join(DB_FILE)).expect("opens");
        own.set_setting("marker", "mine").expect("writes");
        drop(own);
        assert_eq!(adopt(&target, &legacy).expect("no error"), None);
        let own = Db::open(target.join(DB_FILE)).expect("opens");
        assert_eq!(own.setting("marker").expect("reads").as_deref(), Some("mine"));
        assert!(!target.join("attachments").exists());
    }

    #[test]
    fn without_an_old_database_or_in_the_same_folder_nothing_happens() {
        let root = tempfile::tempdir().expect("temp");
        let empty = root.path().join("default");
        fs::create_dir_all(&empty).expect("folder");
        let target = root.path().join("vn.dat.tradelens");
        assert_eq!(adopt(&target, &empty).expect("no error"), None);
        assert!(!target.exists(), "nothing was created");
        let (legacy, _open) = legacy_folder(root.path());
        assert_eq!(adopt(&legacy, &legacy).expect("no error"), None);
    }

    #[test]
    fn an_interrupted_copy_is_started_again() {
        let root = tempfile::tempdir().expect("temp");
        let (legacy, _open) = legacy_folder(root.path());
        let target = root.path().join("vn.dat.tradelens");
        fs::create_dir_all(target.join(STAGING_DIR)).expect("folder");
        fs::write(target.join(STAGING_DIR).join(DB_FILE), b"half a database").expect("junk");
        adopt(&target, &legacy).expect("copies").expect("something to copy");
        let copy = Db::open(target.join(DB_FILE)).expect("the copy opens");
        assert_eq!(copy.setting("marker").expect("reads").as_deref(), Some("from the old folder"));
    }
}
