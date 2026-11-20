//! Microsoft Graph, `/me/drive/special/approot`.

use crate::drive::{CloudDrive, RemoteFile};

pub struct OneDrive {
    pub access_token: String,
}

impl std::fmt::Debug for OneDrive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OneDrive").field("access_token", &"<redacted>").finish()
    }
}

impl CloudDrive for OneDrive {
    fn list(&self, _prefix: &str) -> crate::Result<Vec<RemoteFile>> {
        Err(crate::CloudError::NotImplemented(
            "v0.1: children / PUT content (upload session above 4 MB) / GET content / DELETE",
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
