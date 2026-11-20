//! Google Drive API v3 in `appDataFolder` (`spaces=appDataFolder`).

use crate::drive::{CloudDrive, RemoteFile};

pub struct GoogleDrive {
    pub access_token: String,
}

impl CloudDrive for GoogleDrive {
    fn list(&self, _prefix: &str) -> crate::Result<Vec<RemoteFile>> {
        todo!("v0.1: files.list / files.create (multipart) / files.get?alt=media / files.delete")
    }

    fn upload(&self, _name: &str, _bytes: &[u8]) -> crate::Result<RemoteFile> {
        todo!("v0.1")
    }

    fn download(&self, _file: &RemoteFile) -> crate::Result<Vec<u8>> {
        todo!("v0.1")
    }

    fn delete(&self, _file: &RemoteFile) -> crate::Result<()> {
        todo!("v0.1")
    }
}
