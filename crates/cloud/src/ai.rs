//! AI review of a week's journal: off until the trader turns it on, with their own API
//! key, and sending only what they allow. It looks for patterns the trader may miss
//! (repeated mistakes, setups that stopped working); it never suggests trades.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

/// The model service, called with the trader's own key (kept in the OS credential store).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "provider", rename_all = "snake_case")]
pub enum AiProvider {
    Anthropic {
        model: String,
    },
    OpenAi {
        model: String,
    },
    /// A model on the trader's machine (Ollama); nothing leaves it.
    Local {
        endpoint: String,
        model: String,
    },
}

impl Default for AiProvider {
    fn default() -> Self {
        Self::Anthropic { model: "claude-sonnet-5-5".to_owned() }
    }
}

/// What may be sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disclosure {
    /// Counts, rates and R only.
    #[default]
    Statistics,
    /// Also the trader's notes and review answers, trimmed.
    WithNotes,
}

/// The trader's choices; `enabled` stays false until they turn it on.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AiSettings {
    pub enabled: bool,
    pub provider: AiProvider,
    pub disclosure: Disclosure,
}

/// A week, reduced to what the review needs. Money is left out: R is enough to judge.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WeeklyDigest {
    /// `16/11 – 22/11/2026`.
    pub week: String,
    pub trades: usize,
    pub win_rate: f64,
    pub total_r: f64,
    pub expectancy_r: f64,
    /// `(setup, trades, win rate, total R)`.
    pub setups: Vec<(String, usize, f64, f64)>,
    /// `(mistake, times, R lost)`, the most costly first.
    pub mistakes: Vec<(String, usize, f64)>,
    pub violations: usize,
    pub missed: usize,
    pub missed_r: f64,
    /// Trade notes and review answers; sent only with [`Disclosure::WithNotes`].
    pub notes: Vec<String>,
}

/// The longest note sent, in characters.
const NOTE_LIMIT: usize = 400;

/// The request text for `digest`, in the language of the process (the review comes back in it),
/// within what `disclosure` allows.
pub fn prompt(digest: &WeeklyDigest, disclosure: Disclosure) -> String {
    let mut p = String::new();
    let _ = writeln!(p, "{}", crate::tr!("cloud.ai.prompt.intro", week = digest.week));
    let _ = writeln!(
        p,
        "{}",
        crate::tr!(
            "cloud.ai.prompt.overview",
            trades = digest.trades,
            win_rate = format!("{:.0}", digest.win_rate * 100.0),
            total_r = format!("{:+.2}", digest.total_r),
            expectancy = format!("{:+.2}", digest.expectancy_r),
            violations = digest.violations,
            missed = digest.missed,
            missed_r = format!("{:+.1}", digest.missed_r)
        )
    );
    if !digest.setups.is_empty() {
        let _ = writeln!(p, "{}", crate::tr!("cloud.ai.prompt.by_setup"));
        for (name, trades, win, r) in &digest.setups {
            let _ = writeln!(
                p,
                "{}",
                crate::tr!(
                    "cloud.ai.prompt.setup_line",
                    name = name,
                    trades = trades,
                    win = format!("{:.0}", win * 100.0),
                    r = format!("{r:+.2}")
                )
            );
        }
    }
    if !digest.mistakes.is_empty() {
        let _ = writeln!(p, "{}", crate::tr!("cloud.ai.prompt.mistakes"));
        for (name, times, r) in &digest.mistakes {
            let _ = writeln!(
                p,
                "{}",
                crate::tr!("cloud.ai.prompt.mistake_line", name = name, times = times, r = format!("{r:+.2}"))
            );
        }
    }
    if disclosure == Disclosure::WithNotes && !digest.notes.is_empty() {
        let _ = writeln!(p, "{}", crate::tr!("cloud.ai.prompt.notes"));
        for note in &digest.notes {
            let trimmed: String = note.chars().take(NOTE_LIMIT).collect();
            let _ = writeln!(p, "- {}", trimmed.replace('\n', " "));
        }
    }
    p
}

/// Sends the prompt and returns the review text.
///
/// # Errors
///
/// [`crate::CloudError`] when the AI review is off, the key is missing or the call fails;
/// [`crate::CloudError::NotImplemented`] until v0.3.
pub fn review_week(settings: &AiSettings, _digest: &WeeklyDigest) -> crate::Result<String> {
    if !settings.enabled {
        return Err(crate::CloudError::Auth(crate::tr!("cloud.ai.off")));
    }
    Err(crate::CloudError::NotImplemented("AI review"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest() -> WeeklyDigest {
        WeeklyDigest {
            week: "16/11 – 22/11/2026".into(),
            trades: 14,
            win_rate: 0.5,
            total_r: 2.4,
            expectancy_r: 0.17,
            setups: vec![("Breakout phiên Á".into(), 6, 0.67, 4.1)],
            mistakes: vec![("Dời SL".into(), 2, -2.6)],
            violations: 1,
            missed: 2,
            missed_r: 3.0,
            notes: vec!["Vào US30 sau 21:00 để gỡ lệnh EURUSD".into()],
        }
    }

    #[test]
    fn notes_leave_only_when_allowed_and_money_never() {
        let stats_only = prompt(&digest(), Disclosure::Statistics);
        assert!(stats_only.contains("14 lệnh, win rate 50%, tổng +2.40R"));
        assert!(stats_only.contains("- Dời SL: 2 lần, -2.60R"));
        assert!(!stats_only.contains("US30"));
        assert!(!stats_only.contains('$'));
        assert!(prompt(&digest(), Disclosure::WithNotes).contains("Vào US30 sau 21:00"));
    }

    #[test]
    fn the_review_is_off_by_default() {
        assert!(!AiSettings::default().enabled);
        assert!(review_week(&AiSettings::default(), &digest()).is_err());
    }
}
