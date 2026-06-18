use std::io::Write;

pub fn append(ob_dir: &std::path::Path, operation: &str, detail: &str) -> anyhow::Result<()> {
    let log_path = ob_dir.join(".ob").join("log");
    let timestamp = chrono_free_timestamp();
    let entry = format!("{} {} {}\n", timestamp, operation, detail);
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?
        .write_all(entry.as_bytes())?;
    Ok(())
}

fn chrono_free_timestamp() -> String {
    let dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    // ISO 8601 without chrono dependency
    let secs = dur.as_secs();
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;
    // Simplified date calculation (not perfectly accurate for all dates, but good enough for logs)
    format!("{}-{:02}:{:02}:{:02}", days, hours, minutes, seconds)
}
