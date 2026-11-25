//! The clock a test seeds trades on, for the screens that count days, weeks or months.
//!
//! Its own module, declared only by the tests that seed relative to the clock: `support` is
//! declared by every screen test, and an item it holds that a test does not use is dead code there.

use tradelens_core::Db;

/// Puts every account of `db` on a broker server that runs on UTC.
///
/// A test that seeds trades relative to the clock (`now - n days`) writes them in UTC. Without a
/// known server offset the app takes stored times as the trader's own wall clock, so on a machine
/// ahead of or behind UTC those trades fall on another day than "today" between the two midnights.
/// With the offset at 0 the app knows they are UTC and converts them to the machine's clock, so
/// they are today, this week and this month in whatever zone the machine is. Call it after the
/// batch is stored; an account added later is not touched.
pub(crate) fn on_utc(db: &mut Db) {
    for mut account in db.accounts().expect("accounts read") {
        account.server_offset_min = Some(0);
        db.update_account(&account).expect("the server offset is stored");
    }
}
