//! A cloud drive that lives in memory: for tests of sync between devices, and for the app's demo
//! mode. Clones share the same files, so each device of a test holds its own handle on one drive.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::drive::{CloudDrive, RemoteFile};
use crate::{CloudError, Result};

#[derive(Debug)]
struct Stored {
    id: String,
    bytes: Vec<u8>,
    modified: i64,
}

/// Whether the drive can be reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Link {
    /// Every call works.
    #[default]
    Up,
    /// Every call fails with [`CloudError::Network`].
    Down,
}

#[derive(Debug, Default)]
struct State {
    files: BTreeMap<String, Stored>,
    next_id: u64,
    link: Link,
    uploads: usize,
}

/// An in-memory [`CloudDrive`]. Cloning gives another handle on the same files.
#[derive(Debug, Clone, Default)]
pub struct MemoryDrive {
    state: Arc<Mutex<State>>,
}

impl MemoryDrive {
    /// An empty drive.
    pub fn new() -> Self {
        Self::default()
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("lock poisoned")
    }

    fn online(&self) -> Result<MutexGuard<'_, State>> {
        let state = self.state();
        if state.link == Link::Down {
            Err(CloudError::Network("không có kết nối (ổ đĩa thử nghiệm đang ngắt)".into()))
        } else {
            Ok(state)
        }
    }

    /// Cuts the connection (every call then fails with a network error) or restores it, to test a sync that is interrupted.
    pub fn set_link(&self, link: Link) {
        self.state().link = link;
    }

    /// The names of the files, in order.
    pub fn names(&self) -> Vec<String> {
        self.state().files.keys().cloned().collect()
    }

    /// The content of the file `name`, if it exists.
    pub fn contents(&self, name: &str) -> Option<Vec<u8>> {
        self.state().files.get(name).map(|file| file.bytes.clone())
    }

    /// How many uploads the drive has taken, replacements included.
    pub fn uploads(&self) -> usize {
        self.state().uploads
    }

    /// Puts a file there without going through a device, as another program or a damaged upload would.
    pub fn put(&self, name: &str, bytes: &[u8]) {
        let mut state = self.state();
        store(&mut state, name, bytes);
    }
}

fn store(state: &mut State, name: &str, bytes: &[u8]) -> RemoteFile {
    let modified = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX));
    let id = if let Some(existing) = state.files.get(name) {
        existing.id.clone()
    } else {
        state.next_id += 1;
        format!("memory-{}", state.next_id)
    };
    state.uploads += 1;
    let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    state.files.insert(name.to_owned(), Stored { id: id.clone(), bytes: bytes.to_vec(), modified });
    RemoteFile { id, name: name.to_owned(), size, modified }
}

impl CloudDrive for MemoryDrive {
    fn list(&self, prefix: &str) -> Result<Vec<RemoteFile>> {
        let state = self.online()?;
        Ok(state
            .files
            .iter()
            .filter(|(name, _)| name.starts_with(prefix))
            .map(|(name, file)| RemoteFile {
                id: file.id.clone(),
                name: name.clone(),
                size: u64::try_from(file.bytes.len()).unwrap_or(u64::MAX),
                modified: file.modified,
            })
            .collect())
    }

    fn upload(&self, name: &str, bytes: &[u8]) -> Result<RemoteFile> {
        let mut state = self.online()?;
        Ok(store(&mut state, name, bytes))
    }

    fn download(&self, file: &RemoteFile) -> Result<Vec<u8>> {
        let state = self.online()?;
        state
            .files
            .values()
            .find(|stored| stored.id == file.id)
            .map(|stored| stored.bytes.clone())
            .ok_or_else(|| CloudError::Service { status: 404, message: format!("không có file {}", file.name) })
    }

    fn delete(&self, file: &RemoteFile) -> Result<()> {
        let mut state = self.online()?;
        state.files.retain(|_, stored| stored.id != file.id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_are_listed_by_prefix_replaced_by_name_and_shared_between_handles() {
        let drive = MemoryDrive::new();
        let other = drive.clone();
        let first = drive.upload("changes/a/1-1.json", b"one").expect("uploads");
        drive.upload("snapshots/2026-11-23.db", b"db").expect("uploads");
        let again = other.upload("changes/a/1-1.json", b"two").expect("replaces");
        assert_eq!(first.id, again.id, "the same name is the same file");
        let listed = other.list("changes/").expect("lists");
        assert_eq!(listed.len(), 1);
        assert_eq!(drive.download(&listed[0]).expect("downloads"), b"two");
        drive.delete(&listed[0]).expect("deletes");
        assert_eq!(other.list("changes/").expect("lists").len(), 0, "the file is deleted");
        assert_eq!(other.names(), ["snapshots/2026-11-23.db"]);
    }

    #[test]
    fn an_offline_drive_fails_every_call() {
        let drive = MemoryDrive::new();
        drive.set_link(Link::Down);
        assert!(matches!(drive.list(""), Err(CloudError::Network(_))));
        assert!(matches!(drive.upload("x", b""), Err(CloudError::Network(_))));
        drive.set_link(Link::Up);
        assert!(drive.upload("x", b"").is_ok());
    }
}
