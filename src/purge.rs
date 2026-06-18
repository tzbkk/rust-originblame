use anyhow::Result;
use rayon::prelude::*;
use std::collections::HashSet;
use std::io::Write;
use std::path::Path;

pub struct PurgeResult {
    pub purged: usize,
    pub kept: usize,
    pub archived: usize,
}

pub struct PurgeArchive {
    pub archive_path: std::path::PathBuf,
    pub archived_count: usize,
}

fn archive_timestamp() -> String {
    let dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let total_secs = dur.as_secs();
    let days_since_epoch = total_secs / 86400;
    let time_of_day = total_secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Convert days since 1970-01-01 to year/month/day
    let mut y = 1970;
    let mut remaining = days_since_epoch;

    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        y += 1;
    }

    let leap = is_leap(y);
    let month_days: [u64; 12] = if leap {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut m = 0;
    for &md in &month_days {
        if remaining < md {
            break;
        }
        remaining -= md;
        m += 1;
    }

    format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}",
        y,
        m + 1,
        remaining + 1,
        hours,
        minutes,
        seconds
    )
}

fn is_leap(y: u64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// Returns `None` if `purged_lines` is empty.
fn archive_purged_lines(ob_dir: &Path, purged_lines: &[&[u8]]) -> Result<Option<PurgeArchive>> {
    if purged_lines.is_empty() {
        return Ok(None);
    }

    let archive_dir = ob_dir.join(".ob").join("archive");
    std::fs::create_dir_all(&archive_dir)?;

    let ts = archive_timestamp();
    let archive_path = archive_dir.join(format!("purge.{}", ts));
    let timestamp_str = ts.clone();

    for line in purged_lines {
        let original: serde_json::Value = serde_json::from_slice(line)
            .unwrap_or(serde_json::Value::Null);
        let record = serde_json::json!({
            "line": original,
            "archived_at": timestamp_str,
        });
        crate::storage::jsonl_append(&archive_path, &record)?;
    }

    crate::oplog::append(
        ob_dir,
        "purge_archive",
        &format!("archived {} lines to {:?}", purged_lines.len(), archive_path),
    )?;

    Ok(Some(PurgeArchive {
        archive_path,
        archived_count: purged_lines.len(),
    }))
}

fn load_revoked(ob_dir: &Path) -> Result<Vec<crate::indexer::DocumentRecord>> {
    let mut results = Vec::new();
    let mut seen_hashes: HashSet<String> = HashSet::new();

    let revoked_path = ob_dir
        .join(".ob")
        .join("document-index")
        .join("revoked")
        .join("all");
    if let Some(mmap) = crate::mmap_lines::mmap_file(&revoked_path)? {
        for line in crate::mmap_lines::iter_lines(&mmap) {
            if let Ok(m) = serde_json::from_slice::<crate::indexer::DocumentRecord>(line) {
                if seen_hashes.insert(m.line_hash.clone()) {
                    results.push(m);
                }
            }
        }
    }

    let revoked_section_hashes = find_revoked_section_hashes(ob_dir)?;
    if revoked_section_hashes.is_empty() {
        return Ok(results);
    }

    let section_set: HashSet<&str> = revoked_section_hashes.iter().map(|s| s.as_str()).collect();
    let manifest_dir = ob_dir.join(".ob").join("document-index");
    if manifest_dir.is_dir() {
        for entry in std::fs::read_dir(&manifest_dir)?.filter_map(|e| e.ok()) {
            if !entry.path().is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if name.len() != 2 || !name.chars().all(|c| c.is_ascii_hexdigit()) {
                continue;
            }
            let mmap = match crate::mmap_lines::mmap_file(&entry.path())? {
                Some(m) => m,
                None => continue,
            };
            for line in crate::mmap_lines::iter_lines(&mmap) {
                if let Ok(m) = serde_json::from_slice::<crate::indexer::DocumentRecord>(line) {
                    if m.sources.iter().any(|s| section_set.contains(s.as_str())) {
                        if seen_hashes.insert(m.line_hash.clone()) {
                            results.push(m);
                        }
                    }
                }
            }
        }
    }

    Ok(results)
}

fn find_revoked_section_hashes(ob_dir: &Path) -> Result<Vec<String>> {
    let section_dir = ob_dir.join(".ob").join("sections");
    if !section_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut results = Vec::new();
    for entry in std::fs::read_dir(&section_dir)?.filter_map(|e| e.ok()) {
        if !entry.path().is_file() {
            continue;
        }
        let mmap = match crate::mmap_lines::mmap_file(&entry.path())? {
            Some(m) => m,
            None => continue,
        };
        for line in crate::mmap_lines::iter_lines(&mmap) {
            if let Ok(val) = serde_json::from_slice::<serde_json::Value>(line) {
                let revoked = val
                    .get("revoked")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if revoked {
                    if let Some(sh) = val.get("section_hash").and_then(|v| v.as_str()) {
                        results.push(sh.to_string());
                    }
                }
            }
        }
    }
    Ok(results)
}

fn count_revoked(ob_dir: &Path) -> Result<usize> {
    let path = ob_dir
        .join(".ob")
        .join("document-index")
        .join("revoked")
        .join("all");
    let mmap = match crate::mmap_lines::mmap_file(&path)? {
        Some(m) => m,
        None => return Ok(0),
    };
    Ok(crate::mmap_lines::iter_lines(&mmap).count())
}

fn resolve_author_line_hashes(ob_dir: &Path, author_name: &str) -> Result<Vec<String>> {
    let authors = crate::authors::query(ob_dir, Some(author_name), None)?;
    if authors.is_empty() {
        return Ok(Vec::new());
    }
    let author_ids: Vec<String> = authors.iter().map(|a| a.id.clone()).collect();
    let author_id_refs: HashSet<&str> = author_ids.iter().map(|s| s.as_str()).collect();

    let bin_idx = match crate::binary_index::BinaryIndex::open(ob_dir)? {
        Some(idx) => idx,
        None => {
            return Err(anyhow::anyhow!(
                "binary index not found, run `ob index build` first"
            ))
        }
    };

    let mut section_bucket_prefixes: HashSet<String> = HashSet::new();
    for aid in &author_ids {
        for r in bin_idx.lookup(aid) {
            if let crate::binary_index::IndexRef::DocumentShard { prefix } = r {
                section_bucket_prefixes.insert(format!("{:02x}", prefix));
            }
        }
    }

    let author_id_bytes: Vec<&[u8]> = author_ids.iter().map(|s| s.as_bytes()).collect();
    let mut section_hashes: HashSet<String> = HashSet::new();
    for prefix in &section_bucket_prefixes {
        let section_path = ob_dir.join(".ob").join("sections").join(prefix);
        let matches = crate::storage::scan_lines_matching(&section_path, &author_id_bytes)?;
        for rec in matches {
            if let Some(sh) = rec.get("section_hash").and_then(|v| v.as_str()) {
                if let Some(rec_authors) = rec.get("authors").and_then(|v| v.as_array()) {
                    let has_author = rec_authors
                        .iter()
                        .any(|a| a.as_str().map_or(false, |aid| author_id_refs.contains(aid)));
                    if has_author {
                        section_hashes.insert(sh.to_string());
                    }
                }
            }
        }
    }

    let mut manifest_bucket_prefixes: HashSet<String> = HashSet::new();
    for sh in &section_hashes {
        for r in bin_idx.lookup(sh) {
            if let crate::binary_index::IndexRef::DocumentShard { prefix } = r {
                manifest_bucket_prefixes.insert(format!("{:02x}", prefix));
            }
        }
    }

    let section_hash_bytes: Vec<&[u8]> = section_hashes.iter().map(|s| s.as_bytes()).collect();
    let section_hash_refs: HashSet<&str> = section_hashes.iter().map(|s| s.as_str()).collect();
    let mut results = Vec::new();
    for prefix in &manifest_bucket_prefixes {
        let manifest_path = ob_dir.join(".ob").join("document-index").join(prefix);
        let mmap = match crate::mmap_lines::mmap_file(&manifest_path)? {
            Some(m) => m,
            None => continue,
        };
        for line in crate::mmap_lines::iter_lines(&mmap) {
            let might_match = section_hash_bytes
                .iter()
                .any(|needle| line.windows(needle.len()).any(|w| w == *needle));
            if !might_match {
                continue;
            }
            if let Ok(m) = serde_json::from_slice::<crate::indexer::DocumentRecord>(line) {
                if m.sources
                    .iter()
                    .any(|s| section_hash_refs.contains(s.as_str()))
                {
                    results.push(m.line_hash);
                }
            }
        }
    }

    Ok(results)
}

pub fn purge_revoked(ob_dir: &Path, data_file: &Path, dry_run: bool) -> Result<PurgeResult> {
    if dry_run {
        let count = count_revoked(ob_dir)?;
        return Ok(PurgeResult {
            purged: count,
            kept: 0,
            archived: 0,
        });
    }

    let revoked = load_revoked(ob_dir)?;
    let revoked_hashes: HashSet<String> = revoked.iter().map(|m| m.line_hash.clone()).collect();

    let tmp_path = data_file.with_extension("tmp");
    let mmap = crate::mmap_lines::mmap_file(data_file)?
        .ok_or_else(|| anyhow::anyhow!("data file not found: {}", data_file.display()))?;

    let lines: Vec<&[u8]> = crate::mmap_lines::iter_lines(&mmap).collect();
    let revoked_clone: HashSet<String> = revoked_hashes.clone();
    let should_keep: Vec<bool> = lines
        .par_iter()
        .map(|line| {
            let data: serde_json::Value = match serde_json::from_slice(line) {
                Ok(d) => d,
                Err(_) => return true,
            };
            let hash = crate::hash::compute_hash(&data);
            !revoked_clone.contains(&hash)
        })
        .collect();

    let purged_lines: Vec<&[u8]> = lines
        .iter()
        .zip(should_keep.iter())
        .filter(|(_, keep)| !**keep)
        .map(|(line, _)| *line)
        .collect();

    let archived = match archive_purged_lines(ob_dir, &purged_lines)? {
        Some(a) => a.archived_count,
        None => 0,
    };

    let mut writer = std::io::BufWriter::new(std::fs::File::create(&tmp_path)?);
    let mut purged = 0;
    let mut kept = 0;
    for (i, line) in lines.iter().enumerate() {
        if should_keep[i] {
            writer.write_all(line)?;
            writer.write_all(b"\n")?;
            kept += 1;
        } else {
            purged += 1;
        }
    }
    drop(writer);
    std::fs::rename(&tmp_path, data_file)?;

    crate::oplog::append(
        ob_dir,
        "purge",
        &format!("purged {} lines from {:?}", purged, data_file),
    )?;
    Ok(PurgeResult { purged, kept, archived })
}

pub fn purge_reverse(ob_dir: &Path, data_file: &Path) -> Result<usize> {
    let archive_dir = ob_dir.join(".ob").join("archive");
    if !archive_dir.is_dir() {
        println!("No purge archive found");
        return Ok(0);
    }

    let mut archives: Vec<std::path::PathBuf> = Vec::new();
    for entry in std::fs::read_dir(&archive_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with("purge.") {
            archives.push(entry.path());
        }
    }

    if archives.is_empty() {
        println!("No purge archive found");
        return Ok(0);
    }

    archives.sort_by(|a, b| {
        let sa = a.file_name().unwrap_or_default().to_string_lossy();
        let sb = b.file_name().unwrap_or_default().to_string_lossy();
        sb.cmp(&sa)
    });

    let archive_path = &archives[0];
    let records = crate::storage::jsonl_read(archive_path)?;

    let mut restored = 0;
    for rec in &records {
        if let Some(line_val) = rec.get("line") {
            crate::storage::jsonl_append(data_file, line_val)?;
            restored += 1;
        }
    }

    std::fs::remove_file(archive_path)?;

    crate::oplog::append(
        ob_dir,
        "purge_reverse",
        &format!("restored {} lines from {:?}", restored, archive_path),
    )?;

    Ok(restored)
}

pub fn purge_by_author_indexed(
    ob_dir: &Path,
    author_name: &str,
    data_file: &Path,
    dry_run: bool,
) -> Result<PurgeResult> {
    // Safety check: require prior revoke
    let authors = crate::authors::query(ob_dir, Some(author_name), None)?;
    let is_revoked = authors.iter().any(|a| a.revoked);
    if !is_revoked {
        anyhow::bail!(
            "author '{}' is not revoked. Revoke first with `ob revoke --author '{}'` before purging.",
            author_name,
            author_name
        );
    }

    let line_hashes = resolve_author_line_hashes(ob_dir, author_name)?;
    let line_hash_set: HashSet<String> = line_hashes.iter().cloned().collect();
    let purged_count = line_hash_set.len();

    if dry_run {
        return Ok(PurgeResult {
            purged: purged_count,
            kept: 0,
            archived: 0,
        });
    }

    let tmp_path = data_file.with_extension("tmp");
    let mmap = crate::mmap_lines::mmap_file(data_file)?
        .ok_or_else(|| anyhow::anyhow!("data file not found: {}", data_file.display()))?;

    let lines: Vec<&[u8]> = crate::mmap_lines::iter_lines(&mmap).collect();
    let hash_set_clone: HashSet<String> = line_hash_set.clone();
    let should_keep: Vec<bool> = lines
        .par_iter()
        .map(|line| {
            let data: serde_json::Value = match serde_json::from_slice(line) {
                Ok(d) => d,
                Err(_) => return true,
            };
            let hash = crate::hash::compute_hash(&data);
            !hash_set_clone.contains(&hash)
        })
        .collect();

    let purged_lines: Vec<&[u8]> = lines
        .iter()
        .zip(should_keep.iter())
        .filter(|(_, keep)| !**keep)
        .map(|(line, _)| *line)
        .collect();

    let archived = match archive_purged_lines(ob_dir, &purged_lines)? {
        Some(a) => a.archived_count,
        None => 0,
    };

    let mut writer = std::io::BufWriter::new(std::fs::File::create(&tmp_path)?);
    let mut purged = 0;
    let mut kept = 0;
    for (i, line) in lines.iter().enumerate() {
        if should_keep[i] {
            writer.write_all(line)?;
            writer.write_all(b"\n")?;
            kept += 1;
        } else {
            purged += 1;
        }
    }
    drop(writer);
    std::fs::rename(&tmp_path, data_file)?;

    crate::oplog::append(
        ob_dir,
        "purge",
        &format!(
            "purged {} lines from {:?} (indexed, author={})",
            purged, data_file, author_name
        ),
    )?;
    Ok(PurgeResult { purged, kept, archived })
}
