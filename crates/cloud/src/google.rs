//! Google Drive API v3 in `appDataFolder` (`spaces=appDataFolder`).

use crate::drive::{CloudDrive, RemoteFile};

pub struct GoogleDrive {
    pub access_token: String,
}

impl std::fmt::Debug for GoogleDrive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GoogleDrive").field("access_token", &"<redacted>").finish()
    }
}

impl CloudDrive for GoogleDrive {
    fn list(&self, _prefix: &str) -> crate::Result<Vec<RemoteFile>> {
        Err(crate::CloudError::NotImplemented(
            "v0.1: files.list / files.create (multipart) / files.get?alt=media / files.delete",
        ))
    }

    fn upload(&self, _name: &str, _bytes: &[u8]) -> crate::Result<RemoteFile> {
        Err(crate::CloudError::NotImplemented("v0.1"))
    }

    fn download(&self, _file: &RemoteFile) -> crate::Result<Vec<u8>> {
        Err(crate::CloudError::NotImplemented("v0.1"))
    }

    fn delete(&self, _file: &RemoteFile) -> crate::Result<()> {
        Err(crate::CloudError::NotImplemented("v0.1"))
    }
}
