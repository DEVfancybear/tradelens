//! A [`DealBatch`] as JSON: what the Python script (`python/tradelens_sync.py`) writes with
//! `--out`. The fields are those of [`DealBatch`] and its `Deal`s.

use crate::model::DealBatch;

use super::ImportError;

/// # Errors
///
/// [`ImportError`] when the content is not a readable report.
pub fn parse(bytes: &[u8]) -> Result<DealBatch, ImportError> {
    serde_json::from_slice(bytes).map_err(|e| ImportError::Invalid(e.to_string()))
}

#[cfg(test)]
mod tests {
    use crate::model::{DealEntry, DealKind};

    #[test]
    fn reads_the_bridge_payload() {
        let json = br#"{
            "account": {"login": "52012345", "server": "Demo", "currency": "USD", "server_offset_min": 120},
            "deals": [
                {"ticket": 1, "position_id": 10, "time_msc": 1763650000000, "symbol": "XAUUSD",
                 "type": "buy", "entry": "in", "volume": 0.1, "price": 2650.25, "sl": 2640.0, "risk": 102.5},
                {"ticket": 2, "position_id": 10, "time_msc": 1763653600000, "symbol": "XAUUSD",
                 "type": 1, "entry": 1, "volume": 0.1, "price": 2660.25, "profit": 100.0}
            ]
        }"#;
        let batch = super::parse(json).unwrap();
        assert_eq!(batch.account.server_offset_min, Some(120));
        assert_eq!(batch.deals[0].kind, DealKind::Buy);
        assert_eq!(batch.deals[1].kind, DealKind::Sell);
        assert_eq!(batch.deals[1].entry, DealEntry::Out);
        assert_eq!(batch.deals[0].risk, Some(102.5));
    }
}
