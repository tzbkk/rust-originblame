use anyhow::Result;
use std::path::Path;

const LOG_MAX_SIZE: u64 = 10_485_760; // 10 MiB

pub struct CleanResult {
    pub document_merged: usize,
    pub pid_files_deleted: usize,
    pub archived_records: usize,
    pub log_rotated: usize,
    pub token_index_merged: usize,
    pub token_index_pid_deleted: usize,
}

pub fn clean(ob_dir: &Path, split: bool) -> Result<CleanResult> {
    let ob_dot = ob_dir.join(".ob");
    if !ob_dot.is_dir() {
        return Ok(CleanResult {
            document_merged: 0,
            pid_files_deleted: 0,
            archived_records: 0,
            log_rotated: 0,
            token_index_merged: 0,
            token_index_pid_deleted: 0,
        });
    }

    let mut result = CleanResult {
        document_merged: 0,
        pid_files_deleted: 0,
        archived_records: 0,
        log_rotated: 0,
        token_index_merged: 0,
        token_index_pid_deleted: 0,
    };

    result.log_rotated = rotate_log(ob_dir)?;
    crate::oplog::append(ob_dir, "clean", &format!("split={}", split))?;

    let (merged, deleted) = merge_pid_files(&ob_dot, ob_dir)?;
    result.document_merged = merged;
    result.pid_files_deleted = deleted;

    let tokenizers = crate::token_index::list_tokenizers(ob_dir)?;
    for tokenizer in &tokenizers {
        let (ti_merged, ti_deleted) = crate::token_index::merge_pid_files(ob_dir, tokenizer)?;
        result.token_index_merged += ti_merged;
        result.token_index_pid_deleted += ti_deleted;

        // Token-index is position-indexed: entry i maps to token range
        // [cumsum(token_count[0..i]), cumsum(token_count[0..i+1])) in the
        // packed binary.  Archiving (removing) entries would shift positions
        // and break this mapping, so token-index entries are never archived.
        // Revoke only flips the `revoked` flag in-place; the bitmask produced
        // by generate-set tells the unlearning algorithm which entries to skip.

        let _ = crate::token_index::build_binary_index(ob_dir, tokenizer);
    }

    result.archived_records += archive_revoked(ob_dir)?;

    if split {
        let split_dir = ob_dir.join(".ob").join("split");
        if split_dir.is_dir() {
            let _ = std::fs::remove_dir_all(&split_dir);
            let _ = std::fs::create_dir_all(&split_dir);
        }
    }

    Ok(result)
}

fn rotate_log(ob_dir: &Path) -> Result<usize> {
    let log_path = ob_dir.join(".ob").join("log");
    if !log_path.exists() {
        return Ok(0);
    }
    let metadata = std::fs::metadata(&log_path)?;
    if metadata.len() > LOG_MAX_SIZE {
        let log_1 = ob_dir.join(".ob").join("log.1");
        std::fs::rename(&log_path, &log_1)?;
        std::fs::write(&log_path, "")?;
        return Ok(1);
    }
    Ok(0)
}

fn is_hex_bucket(s: &str) -> bool {
    s.len() == 2 && s.chars().all(|c| c.is_ascii_hexdigit())
}

fn merge_pid_files(ob_dot: &Path, ob_dir: &Path) -> Result<(usize, usize)> {
    let mut pid_files: Vec<std::path::PathBuf> = Vec::new();

    for entry in std::fs::read_dir(ob_dot)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(suffix) = name.strip_prefix("docidx.") {
            if !is_hex_bucket(suffix) && entry.path().is_file() {
                pid_files.push(entry.path());
            }
        }
    }

    let mut merged = 0;
    for path in &pid_files {
        let records = crate::storage::jsonl_read(path)?;
        for record in &records {
            if let Some(line_hash) = record.get("line_hash").and_then(|v| v.as_str()) {
                if !line_hash.is_empty() {
                    let shard = crate::storage::shard_path(ob_dir, "document-index", line_hash);
                    crate::storage::jsonl_append(&shard, record)?;
                    merged += 1;
                }
            }
        }
    }

    let mut deleted = 0;
    for path in &pid_files {
        if path.exists() {
            std::fs::remove_file(path)?;
            deleted += 1;
        }
    }

    Ok((merged, deleted))
}

fn archive_revoked(ob_dir: &Path) -> Result<usize> {
    let archive_dir = ob_dir.join(".ob").join("archive");
    std::fs::create_dir_all(&archive_dir)?;
    let mut count = 0;

    count += archive_layer(ob_dir, &archive_dir, "document-index", "line_hash")?;
    count += archive_layer(ob_dir, &archive_dir, "sections", "section_hash")?;
    count += archive_layer(ob_dir, &archive_dir, "authors", "id")?;

    Ok(count)
}

fn archive_layer(ob_dir: &Path, archive_dir: &Path, layer: &str, hash_key: &str) -> Result<usize> {
    let layer_dir = ob_dir.join(".ob").join(layer);
    if !layer_dir.is_dir() {
        return Ok(0);
    }
    let mut count = 0;
    for entry in std::fs::read_dir(&layer_dir)? {
        let entry = entry?;
        if !entry.path().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.len() != 2 {
            continue;
        }
        for record in crate::storage::jsonl_read(&entry.path())? {
            let revoked = record
                .get("revoked")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let orphan = record
                .get("orphan")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if !revoked && !orphan {
                continue;
            }
            if let Some(hash_val) = record.get(hash_key).and_then(|v| v.as_str()) {
                if !hash_val.is_empty() {
                    let bucket = &hash_val[..2];
                    let archive_path = archive_dir.join(format!("{}.{}", layer, bucket));
                    crate::storage::jsonl_append(&archive_path, &record)?;
                    count += 1;
                }
            }
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_returns_zero_when_no_ob_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let result = clean(tmp.path(), false).unwrap();
        assert_eq!(result.document_merged, 0);
        assert_eq!(result.pid_files_deleted, 0);
        assert_eq!(result.archived_records, 0);
        assert_eq!(result.log_rotated, 0);
        assert_eq!(result.token_index_merged, 0);
        assert_eq!(result.token_index_pid_deleted, 0);
    }

    #[test]
    fn clean_merges_token_index_pid_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ob_dir = tmp.path();
        let ob_dot = ob_dir.join(".ob");
        std::fs::create_dir_all(&ob_dot).unwrap();

        crate::token_index::write_pid(ob_dir, "gpt2", 10, &["src1".to_string()]).unwrap();
        crate::token_index::write_pid(ob_dir, "gpt2", 20, &["src2".to_string()]).unwrap();
        crate::token_index::write_pid(ob_dir, "llama", 30, &["src3".to_string()]).unwrap();

        let result = clean(ob_dir, false).unwrap();
        assert_eq!(result.token_index_merged, 3);
        assert_eq!(result.token_index_pid_deleted, 2);

        assert!(crate::token_index::list_pid_files(ob_dir, "gpt2").unwrap().is_empty());
        assert!(crate::token_index::list_pid_files(ob_dir, "llama").unwrap().is_empty());

        let gpt2 = crate::token_index::read_merged(ob_dir, "gpt2").unwrap();
        assert_eq!(gpt2.len(), 2);
        let llama = crate::token_index::read_merged(ob_dir, "llama").unwrap();
        assert_eq!(llama.len(), 1);
    }

    #[test]
    fn clean_preserves_revoked_token_index_entries() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ob_dir = tmp.path();
        let ob_dot = ob_dir.join(".ob");
        std::fs::create_dir_all(&ob_dot).unwrap();

        crate::token_index::write_pid(ob_dir, "gpt2", 10, &["src1".to_string()]).unwrap();
        crate::token_index::write_pid(ob_dir, "gpt2", 20, &["src2".to_string()]).unwrap();

        crate::token_index::merge_pid_files(ob_dir, "gpt2").unwrap();

        let mut targets = std::collections::HashSet::new();
        targets.insert("src1".to_string());
        crate::token_index::revoke_by_sources(ob_dir, "gpt2", &targets).unwrap();

        let result = clean(ob_dir, false).unwrap();
        assert_eq!(result.token_index_merged, 0);
        assert_eq!(result.token_index_pid_deleted, 0);

        // Token-index entries are never archived — position-indexed design
        let entries = crate::token_index::read_merged(ob_dir, "gpt2").unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries[0].revoked);
        assert!(!entries[1].revoked);
    }
}
