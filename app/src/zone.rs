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

/// Today as `clock` counts days, for a server that is `server_offset_min` ahead of UTC.
///
/// "Now" is first moved onto the server's wall clock (what MT5 times are on), then onto
/// `clock`: the server's own day for [`TimeDisplay::Server`], the trader's for
/// [`TimeDisplay::Local`]. Without a known offset the stored times are read as UTC, so is "now".
pub(crate) fn today_on(clock: Clock, server_offset_min: Option<i32>, now_utc_ms: i64) -> NaiveDate {
    let now_server = now_utc_ms + server_offset_min.map_or(0, |minutes| i64::from(minutes) * 60_000);
    clock.day(now_server)
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
    fn without_a_known_offset_both_clocks_read_utc() {
        let local = Zone { display: TimeDisplay::Local, server_offset_min: None, local_offset_min: 420 };
        let server = Zone { display: TimeDisplay::Server, ..local };
        assert_eq!(local.today(LATE_UTC_MS), date(2026, 11, 20));
        assert_eq!(server.today(LATE_UTC_MS), date(2026, 11, 20));
    }

    #[test]
    fn the_setting_reads_local_unless_server_is_stored() {
        let mut db = Db::in_memory().expect("a memory database opens");
        assert_eq!(time_display(&db).expect("reads"), TimeDisplay::Local);
        db.set_setting(TIME_DISPLAY_KEY, "server").expect("stored");
        assert_eq!(time_display(&db).expect("reads"), TimeDisplay::Server);
    }
}
