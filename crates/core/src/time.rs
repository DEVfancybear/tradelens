//! Times. MT5 reports the broker server's wall clock, with no zone. The journal keeps
//! those times as they are, and shifts them to the trader's zone for display and for
//! grouping by day, week and hour when the server's offset is known.

use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, Timelike, Weekday};

/// How server times become the trader's local times.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clock {
    /// Minutes the server is ahead of UTC; `None` keeps server times as they are.
    pub server_offset_min: Option<i32>,
    /// Minutes the trader is ahead of UTC (Vietnam: 420).
    pub local_offset_min: i32,
}

impl Default for Clock {
    fn default() -> Self {
        Self { server_offset_min: None, local_offset_min: 420 }
    }
}

impl Clock {
    /// The local wall clock of a server time.
    pub fn local(&self, server_ms: i64) -> NaiveDateTime {
        let shifted = match self.server_offset_min {
            Some(server) => server_ms + i64::from(self.local_offset_min - server) * 60_000,
            None => server_ms,
        };
        naive(shifted)
    }

    /// The UTC wall clock of a server time (for trading sessions).
    pub fn utc(&self, server_ms: i64) -> NaiveDateTime {
        let shifted = server_ms - i64::from(self.server_offset_min.unwrap_or(0)) * 60_000;
        naive(shifted)
    }

    pub fn day(&self, server_ms: i64) -> NaiveDate {
        self.local(server_ms).date()
    }
}

/// Which clock the pages show times on (a setting of the trader's).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimeDisplay {
    /// The trader's own zone: server times are shifted when the server's offset is known.
    #[default]
    Local,
    /// The broker server's wall clock, as MT5 shows it.
    Server,
}

impl TimeDisplay {
    /// The text the setting is stored as.
    pub fn key(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Server => "server",
        }
    }

    /// The choice stored as `text`; anything else reads as the default (local).
    pub fn parse(text: &str) -> Self {
        if text.trim() == Self::Server.key() { Self::Server } else { Self::Local }
    }

    /// The clock of an account whose server is `server_offset_min` ahead of UTC.
    pub fn clock(self, server_offset_min: Option<i32>, local_offset_min: i32) -> Clock {
        match self {
            Self::Local => Clock { server_offset_min, local_offset_min },
            Self::Server => Clock { server_offset_min: None, local_offset_min },
        }
    }
}

pub fn naive(ms: i64) -> NaiveDateTime {
    DateTime::from_timestamp_millis(ms).map_or_default(|d| d.naive_utc())
}

/// Milliseconds of a wall clock read as UTC.
pub fn to_ms(dt: NaiveDateTime) -> i64 {
    dt.and_utc().timestamp_millis()
}

/// Parses MT5's `2026.11.20 14:05:33` (or `-` separated, seconds optional).
pub fn parse_mt5_time(s: &str) -> Option<i64> {
    let s = s.trim().replace('-', ".");
    for format in ["%Y.%m.%d %H:%M:%S%.f", "%Y.%m.%d %H:%M:%S", "%Y.%m.%d %H:%M", "%Y.%m.%d"] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(&s, format) {
            return Some(to_ms(dt));
        }
        if format == "%Y.%m.%d"
            && let Ok(d) = NaiveDate::parse_from_str(&s, format)
        {
            return Some(to_ms(d.and_hms_opt(0, 0, 0)?));
        }
    }
    None
}

pub fn format_time(dt: NaiveDateTime) -> String {
    dt.format("%d/%m/%Y %H:%M").to_string()
}

pub fn format_day(d: NaiveDate) -> String {
    d.format("%d/%m/%Y").to_string()
}

pub fn weekday_vi(d: Weekday) -> &'static str {
    match d {
        Weekday::Mon => "Thứ 2",
        Weekday::Tue => "Thứ 3",
        Weekday::Wed => "Thứ 4",
        Weekday::Thu => "Thứ 5",
        Weekday::Fri => "Thứ 6",
        Weekday::Sat => "Thứ 7",
        Weekday::Sun => "Chủ nhật",
    }
}

/// The forex session a UTC hour opens in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Session {
    Asia,
    London,
    Overlap,
    NewYork,
    Late,
}

impl Session {
    pub fn of_utc_hour(hour: u32) -> Self {
        match hour {
            0..=6 => Self::Asia,
            7..=11 => Self::London,
            12..=15 => Self::Overlap,
            16..=20 => Self::NewYork,
            _ => Self::Late,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Asia => "Phiên Á (00–07 UTC)",
            Self::London => "Phiên Âu (07–12 UTC)",
            Self::Overlap => "Âu–Mỹ giao thoa (12–16 UTC)",
            Self::NewYork => "Phiên Mỹ (16–21 UTC)",
            Self::Late => "Cuối ngày (21–24 UTC)",
        }
    }

    pub const ALL: [Self; 5] = [Self::Asia, Self::London, Self::Overlap, Self::NewYork, Self::Late];
}

pub fn hour_of(dt: NaiveDateTime) -> u32 {
    dt.hour()
}

pub fn weekday_index(d: NaiveDate) -> u32 {
    d.weekday().num_days_from_monday()
}

pub fn format_duration(secs: i64) -> String {
    let secs = secs.max(0);
    let (d, h, m) = (secs / 86_400, secs % 86_400 / 3600, secs % 3600 / 60);
    if d > 0 {
        format!("{d}n {h}g")
    } else if h > 0 {
        format!("{h}g {m}p")
    } else if m > 0 {
        format!("{m}p")
    } else {
        format!("{secs}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mt5_times() {
        let ms = parse_mt5_time("2026.11.20 14:05:33").unwrap();
        assert_eq!(format_time(naive(ms)), "20/11/2026 14:05");
        assert!(parse_mt5_time("2026-11-20 14:05").is_some());
        assert!(parse_mt5_time("hello").is_none());
    }

    #[test]
    fn the_display_choice_picks_the_clock() {
        let ms = parse_mt5_time("2026.11.20 22:30:00").unwrap();
        let local = TimeDisplay::Local.clock(Some(120), 420);
        let server = TimeDisplay::Server.clock(Some(120), 420);
        assert_eq!(format_time(local.local(ms)), "21/11/2026 03:30");
        assert_eq!(format_time(server.local(ms)), "20/11/2026 22:30");
        assert_eq!(TimeDisplay::parse("server"), TimeDisplay::Server);
        assert_eq!(TimeDisplay::parse("anything"), TimeDisplay::Local);
        assert_eq!(TimeDisplay::parse(TimeDisplay::Server.key()), TimeDisplay::Server);
    }

    #[test]
    fn shifts_server_time_to_local() {
        let clock = Clock { server_offset_min: Some(120), local_offset_min: 420 };
        let ms = parse_mt5_time("2026.11.20 22:30:00").unwrap();
        // 22:30 at UTC+2 is 03:30 next day at UTC+7.
        assert_eq!(format_time(clock.local(ms)), "21/11/2026 03:30");
        assert_eq!(clock.utc(ms).hour(), 20);
    }
}
