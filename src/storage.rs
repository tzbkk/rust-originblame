use std::io::Write;

pub fn jsonl_lines_raw(path: &std::path::Path) -> anyhow::Result<Option<memmap2::Mmap>> {
    crate::mmap_lines::mmap_file(path)
}

pub fn jsonl_read(path: &std::path::Path) -> anyhow::Result<Vec<serde_json::Value>> {
    let mmap = match jsonl_lines_raw(path)? {
        Some(m) => m,
        None => return Ok(Vec::new()),
    };
    let mut records = Vec::new();
    for line in crate::mmap_lines::iter_lines(&mmap) {
        if let Ok(v) = serde_json::from_slice(line) {
            records.push(v);
        }
    }
    Ok(records)
}

pub fn jsonl_append(path: &std::path::Path, record: &serde_json::Value) -> anyhow::Result<()> {
    let line = serde_json::to_string(record)?;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(format!("{}\n", line).as_bytes())?;
    Ok(())
}

pub fn shard_path(ob_dir: &std::path::Path, layer: &str, hash_hex: &str) -> std::path::PathBuf {
    let bucket = &hash_hex[..2];
    ob_dir.join(".ob").join(layer).join(bucket)
}

pub fn shard_read(
    ob_dir: &std::path::Path,
    layer: &str,
    hash_hex: &str,
) -> anyhow::Result<Vec<serde_json::Value>> {
    jsonl_read(&shard_path(ob_dir, layer, hash_hex))
}

pub fn scan_lines_matching(
    path: &std::path::Path,
    needles: &[&[u8]],
) -> anyhow::Result<Vec<serde_json::Value>> {
    let mmap = match jsonl_lines_raw(path)? {
        Some(m) => m,
        None => return Ok(Vec::new()),
    };
    let mut results = Vec::new();
    for line in crate::mmap_lines::iter_lines(&mmap) {
        let might_match = needles
            .iter()
            .any(|needle| line.windows(needle.len()).any(|window| window == *needle));
        if !might_match {
            continue;
        }
        if let Ok(v) = serde_json::from_slice(line) {
            results.push(v);
        }
    }
    Ok(results)
}

pub fn shard_iterate_all(
    ob_dir: &std::path::Path,
    layer: &str,
) -> anyhow::Result<Vec<serde_json::Value>> {
    let layer_dir = ob_dir.join(".ob").join(layer);
    if !layer_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut all = Vec::new();
    for entry in std::fs::read_dir(&layer_dir)? {
        let entry = entry?;
        if !entry.path().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.len() != 2 || !name.chars().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        all.extend(jsonl_read(&entry.path())?);
    }
    Ok(all)
}
