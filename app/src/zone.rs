//! The clock times are shown on, in one place: the trader's `time_display` setting, the
//! machine's zone and the broker server's offset make a [`Zone`] that the snapshot carries and
//! every page reads its trade times and "today" from. Pure over a [`Db`] and a time, so it is
//! tested without a window.

use chrono::NaiveDate;
use tradelens_core::{
    Db, DbError,
    time::{Clock, TimeDisplay},
};

/// The setting that holds which clock times are shown on (`local` or `server`).
pub(crate) const TIME_DISPLAY_KEY: &str = "time_display";

/// The clock choice the trader made in the settings.
///
/// # Errors
///
/// [`DbError`] when the setting cannot be read.
pub(crate) fn time_display(db: &Db) -> Result<TimeDisplay, DbError> {
    Ok(db.setting(TIME_DISPLAY_KEY)?.map_or_else(TimeDisplay::default, |text| TimeDisplay::parse(&text)))
}

/// Minutes the machine's zone is ahead of UTC.
pub(crate) fn local_offset_min() -> i32 {
    chrono::Local::now().offset().local_minus_utc() / 60
}

/// "Now" on the scale stored trade times are on.
///
/// With a known server offset the stored times are the broker server's wall clock (MT5 reports no
/// zone), so "now" is moved onto it. Without one, nothing says which zone a stored time is in:
/// what the trader types in (manual entry) or imports (Notion, spreadsheets) is their own wall
/// clock, kept as it was written (`Clock::server_ms` and `Clock::local` leave such times
/// unshifted). "Now" is then the machine's wall clock too, so a trade entered now falls on today
/// and the days trades are grouped into agree with the day the trader sees.
pub(crate) fn stored_now(server_offset_min: Option<i32>, local_offset_min: i32, now_utc_ms: i64) -> i64 {
    let ahead_of_utc = server_offset_min.unwrap_or(local_offset_min);
    now_utc_ms + i64::from(ahead_of_utc) * 60_000
}

/// Today as `clock` counts days, for a server that is `server_offset_min` ahead of UTC.
///
/// "Now" is first put on the scale of the stored times (see [`stored_now`]: the server's wall
/// clock, or the trader's when the offset is unknown), then onto `clock`: the server's own day for
/// [`TimeDisplay::Server`], the trader's for [`TimeDisplay::Local`]. Without a known offset both
/// displays read the trader's own day, because those times were never on a server's clock.
pub(crate) fn today_on(clock: Clock, server_offset_min: Option<i32>, now_utc_ms: i64) -> NaiveDate {
    clock.day(stored_now(server_offset_min, clock.local_offset_min, now_utc_ms))
}

/// How server times become the ones shown, for the accounts in view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Zone {
    /// Which clock the trader chose.
    pub(crate) display: TimeDisplay,
    /// Minutes the server is ahead of UTC, whatever the display: the first known one of the
    /// accounts in view.
    pub(crate) server_offset_min: Option<i32>,
    /// Minutes the machine is ahead of UTC.
    pub(crate) local_offset_min: i32,
}

impl Default for Zone {
    fn default() -> Self {
        Self { display: TimeDisplay::default(), server_offset_min: None, local_offset_min: local_offset_min() }
    }
}

impl Zone {
    /// The clock times are shown and days are grouped on.
    pub(crate) fn clock(self) -> Clock {
        self.display.clock(self.server_offset_min, self.local_offset_min)
    }

    /// The date today is, on [`Zone::clock`].
    pub(crate) fn today(self, now_utc_ms: i64) -> NaiveDate {
        today_on(self.clock(), self.server_offset_min, now_utc_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tradelens_core::time;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("a real date")
    }

    /// 2026-11-20 22:30 UTC.
    const LATE_UTC_MS: i64 = 1_795_213_800_000;

    #[test]
    fn today_is_the_traders_day_or_the_servers() {
        // The server is at UTC+2 (00:30 on the 21st), the trader at UTC+7 (05:30 on the 21st).
        let local = Zone { display: TimeDisplay::Local, server_offset_min: Some(120), local_offset_min: 420 };
        assert_eq!(local.today(LATE_UTC_MS), date(2026, 11, 21));
        let server = Zone { display: TimeDisplay::Server, ..local };
        assert_eq!(server.today(LATE_UTC_MS), date(2026, 11, 21));
        // A trader behind the server sees the day before it.
        let behind = Zone { display: TimeDisplay::Local, server_offset_min: Some(120), local_offset_min: -300 };
        assert_eq!(behind.today(LATE_UTC_MS), date(2026, 11, 20));
        assert_eq!(Zone { display: TimeDisplay::Server, ..behind }.today(LATE_UTC_MS), date(2026, 11, 21));
    }

    #[test]
    fn without_a_known_offset_today_is_the_machines_day_on_both_clocks() {
        // 22:30 UTC on the 20th is 05:30 on the 21st at UTC+7: the trader is already on the 21st.
        let local = Zone { display: TimeDisplay::Local, server_offset_min: None, local_offset_min: 420 };
        let server = Zone { display: TimeDisplay::Server, ..local };
        assert_eq!(local.today(LATE_UTC_MS), date(2026, 11, 21));
        assert_eq!(server.today(LATE_UTC_MS), date(2026, 11, 21));
        // A trader behind UTC is still on the 20th (15:30 at UTC-7).
        let behind = Zone { local_offset_min: -420, ..local };
        assert_eq!(behind.today(LATE_UTC_MS), date(2026, 11, 20));
        assert_eq!(Zone { display: TimeDisplay::Server, ..behind }.today(LATE_UTC_MS), date(2026, 11, 20));
    }

    #[test]
    fn a_trade_entered_now_falls_on_today_whatever_the_zone_and_hour() {
        // Entered as the machine's wall clock reads it (what the entry form stores without an
        // offset), the trade is grouped on the very day "today" names.
        let midnight_utc = LATE_UTC_MS - 22 * 3_600_000 - 30 * 60_000;
        for local_offset_min in [-720, -300, 0, 120, 420, 840] {
            let zone = Zone { display: TimeDisplay::Local, server_offset_min: None, local_offset_min };
            for hour in 0..24 {
                let now_utc_ms = midnight_utc + i64::from(hour) * 3_600_000;
                let wall_clock = time::naive(now_utc_ms + i64::from(local_offset_min) * 60_000);
                let stored = zone.clock().server_ms(wall_clock);
                assert_eq!(
                    zone.clock().day(stored),
                    zone.today(now_utc_ms),
                    "UTC{local_offset_min:+} min at {hour}:00 UTC"
                );
            }
        }
    }

    #[test]
    fn stored_now_is_the_servers_clock_when_known_else_the_traders() {
        assert_eq!(stored_now(Some(120), 420, 1_000_000), 1_000_000 + 7_200_000);
        assert_eq!(stored_now(None, 420, 1_000_000), 1_000_000 + 25_200_000);
        assert_eq!(stored_now(None, 0, 1_000_000), 1_000_000);
    }

    #[test]
    fn the_setting_reads_local_unless_server_is_stored() {
        let mut db = Db::in_memory().expect("a memory database opens");
        assert_eq!(time_display(&db).expect("reads"), TimeDisplay::Local);
        db.set_setting(TIME_DISPLAY_KEY, "server").expect("stored");
        assert_eq!(time_display(&db).expect("reads"), TimeDisplay::Server);
    }
}
