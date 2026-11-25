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

/// A drive is shared between the threads that move the images, hence `Send + Sync`.
pub trait CloudDrive: Send + Sync {
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
    /// Creates the file `name`, which the caller knows the drive does not hold (it has just listed
    /// the folder). A drive that must look a name up before it can replace it skips that request
    /// here; the default is [`CloudDrive::upload`]. Called with a name that exists, a drive may
    /// end up with two files of that name, so only call it after listing.
    ///
    /// # Errors
    ///
    /// [`crate::CloudError::Network`] or [`crate::CloudError::Service`] when the request fails.
    fn upload_new(&self, name: &str, bytes: &[u8]) -> crate::Result<RemoteFile> {
        self.upload(name, bytes)
    }
    /// # Errors
    ///
    /// [`crate::CloudError::Network`] or [`crate::CloudError::Service`] when the request fails.
    fn download(&self, file: &RemoteFile) -> crate::Result<Vec<u8>>;
    /// # Errors
    ///
    /// [`crate::CloudError::Network`] or [`crate::CloudError::Service`] when the request fails.
    fn delete(&self, file: &RemoteFile) -> crate::Result<()>;
}
