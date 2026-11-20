//! The Notion API, for importing a trading journal database without exporting it:
//! an internal integration token, the databases shared with it, their rows.
//! Rows are mapped with [`tradelens_core::import::notion::NotionMapping`].

/// A database shared with the integration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotionDatabase {
    pub id: String,
    pub title: String,
    /// Property names, as the mapping screen lists them.
    pub properties: Vec<String>,
}

/// The API, `https://api.notion.com/v1`, `Notion-Version: 2022-06-28`.
pub struct NotionClient {
    token: String,
}

impl std::fmt::Debug for NotionClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NotionClient").field("token", &"<redacted>").finish()
    }
}

impl NotionClient {
    /// A client for the integration token (`secret_…` / `ntn_…`), kept in the OS store.
    pub fn new(token: String) -> Self {
        Self { token }
    }

    /// The databases shared with the integration (`POST /search`, filter `database`).
    ///
    /// # Errors
    ///
    /// [`crate::CloudError`] when the token is refused or the request fails.
    pub fn databases(&self) -> crate::Result<Vec<NotionDatabase>> {
        let _ = &self.token;
        Err(crate::CloudError::NotImplemented("Notion search"))
    }

    /// Every row of a database as `(property, plain text)` pairs
    /// (`POST /databases/{id}/query`, paginated by `start_cursor`).
    ///
    /// # Errors
    ///
    /// [`crate::CloudError`] when the database is not shared or the request fails.
    pub fn rows(&self, _database_id: &str) -> crate::Result<Vec<Vec<(String, String)>>> {
        Err(crate::CloudError::NotImplemented("Notion query"))
    }
}
