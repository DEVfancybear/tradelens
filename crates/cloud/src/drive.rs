//! What sync needs from a cloud drive: a flat folder of named files, app-only.

/// A file in the app folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFile {
    /// The provider's id.
    pub id: String,
    /// `changes/<device>/<seq>.json`, `snapshots/<date>.db.zst`, `attachments/<hash>.png`.
    pub name: String,
    pub size: u64,
    /// Unix seconds.
    pub modified: i64,
}

pub trait CloudDrive {
    /// Files whose name starts with `prefix`.
    ///
    /// # Errors
    ///
    /// [`crate::CloudError::Network`] or [`crate::CloudError::Service`] when the request fails.
    fn list(&self, prefix: &str) -> crate::Result<Vec<RemoteFile>>;
    /// Creates or replaces the file `name`.
    ///
    /// # Errors
    ///
    /// [`crate::CloudError::Network`] or [`crate::CloudError::Service`] when the request fails.
    fn upload(&self, name: &str, bytes: &[u8]) -> crate::Result<RemoteFile>;
    /// # Errors
    ///
    /// [`crate::CloudError::Network`] or [`crate::CloudError::Service`] when the request fails.
    fn download(&self, file: &RemoteFile) -> crate::Result<Vec<u8>>;
    /// # Errors
    ///
    /// [`crate::CloudError::Network`] or [`crate::CloudError::Service`] when the request fails.
    fn delete(&self, file: &RemoteFile) -> crate::Result<()>;
}
