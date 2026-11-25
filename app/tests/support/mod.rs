//! What the screen tests share: a data directory that cleans up after itself, the one timeout they
//! wait with, and a wait for what a background write stores.
//!
//! Every wait is a poll for a condition, never a sleep for a guessed time. `WAIT` is the longest any
//! of them may take: generous, because a loaded CI machine runs these tests next to many others, and
//! it costs nothing when the condition holds sooner.

use std::{
    path::Path,
    time::{Duration, Instant},
};

use nsh_core::{Size, prelude::px};
use nsh_engine::EngineConfig;
use nsh_testing::Headless;
use tradelens_core::Db;

/// The longest any wait of a screen test lasts.
pub(crate) const WAIT: Duration = Duration::from_secs(30);

/// How often a condition outside the app (the database, a file) is looked at.
const POLL: Duration = Duration::from_millis(25);

/// A data directory holding the app's database, and the engine's view of it.
#[derive(Debug)]
pub(crate) struct Fixture {
    /// Taken by `Drop`, which removes the directory itself.
    pub(crate) dir: Option<tempfile::TempDir>,
}

impl Fixture {
    pub(crate) fn new() -> Self {
        let dir = tempfile::tempdir().expect("a temp dir");
        // The app opens the database the tests seed and read.
        Db::open(dir.path().join("tradelens.db")).expect("the database opens");
        Self { dir: Some(dir) }
    }

    /// The data directory.
    pub(crate) fn path(&self) -> &Path {
        self.dir.as_ref().expect("the directory lives until the fixture drops").path()
    }

    /// A fresh connection to the app's database.
    pub(crate) fn db(&self) -> Db {
        Db::open(self.path().join("tradelens.db")).expect("the database opens")
    }

    /// Starts the app on a window of `width` x `height` with this fixture as its data directory;
    /// `tweak` may change the rest of the engine's configuration. Returns once the app has opened.
    pub(crate) fn start(&self, width: f32, height: f32, tweak: impl FnOnce(&mut EngineConfig)) -> Headless {
        let mut config = EngineConfig {
            viewport: Size::new(px(width), px(height)),
            data_dir: Some(self.path().to_path_buf()),
            ..EngineConfig::new(nsh_testing::windowed_root::<nsh_app_tradelens::Workspace>())
        };
        tweak(&mut config);
        let mut app = Headless::start(config).expect("the app starts");
        app.wait_for("Tổng quan").expect("the app opens");
        app
    }
}

impl Drop for Fixture {
    /// Removes the directory. The engine closes the database on its own thread a moment after the
    /// test drops the app, and Windows cannot delete an open file, so removal is retried until the
    /// database is closed (`tempfile`'s own drop tries once and leaves the directory behind).
    fn drop(&mut self) {
        let Some(dir) = self.dir.take() else { return };
        let path = dir.keep();
        let started = Instant::now();
        loop {
            match std::fs::remove_dir_all(&path) {
                Ok(()) => return,
                Err(_) if !path.exists() => return,
                Err(error) if started.elapsed() >= WAIT => {
                    assert!(
                        std::thread::panicking(),
                        "could not remove the test directory {} within {WAIT:?}: {error}",
                        path.display()
                    );
                    return;
                }
                Err(_) => std::thread::sleep(POLL),
            }
        }
    }
}

/// Waits until `check` holds, looking at most for [`WAIT`]; for what a background write stores
/// (the database, a file) and the screen cannot show.
pub(crate) fn wait_until(what: &str, check: impl Fn() -> bool) {
    let started = Instant::now();
    while !check() {
        assert!(started.elapsed() < WAIT, "{what} did not happen within {WAIT:?}");
        std::thread::sleep(POLL);
    }
}
