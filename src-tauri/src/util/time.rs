use chrono::{TimeZone, Utc};

pub fn utc_now() -> String {
    Utc::now().to_rfc3339()
}

pub fn epoch_to_utc(epoch: i64) -> String {
    Utc.timestamp_opt(epoch, 0)
        .single()
        .unwrap_or_else(Utc::now)
        .to_rfc3339()
}
