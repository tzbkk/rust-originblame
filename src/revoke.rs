use anyhow::Result;
use std::collections::HashSet;
use std::io::Write;
use std::path::Path;

pub fn revoke_section(ob_dir: &Path, section_hash: &str, reverse: bool) -> Result<usize> {
    let bucket = &section_hash[..2];
    let shard_path = ob_dir.join(".ob").join("sections").join(bucket);
    if !shard_path.exists() {
        return Ok(0);
    }
    let records = crate::storage::jsonl_read(&shard_path)?;
    let mut count = 0;
    let mut updated = Vec::new();
    for mut rec in records {
        if rec.get("section_hash").and_then(|v| v.as_str()) == Some(section_hash) {
            let revoked = rec
                .get("revoked")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            rec.as_object_mut()
                .map(|m| m.insert("revoked".to_string(), serde_json::Value::Bool(!revoked)));
            count += 1;
        }
        updated.push(rec);
    }
    if count > 0 {
        let mut file = std::fs::File::create(&shard_path)?;
        for rec in &updated {
            writeln!(file, "{}", serde_json::to_string(rec)?)?;
        }
    }
    let op_name = if reverse {
        "revoke_reverse_section"
    } else {
        "revoke_section"
    };
    crate::oplog::append(
        ob_dir,
        op_name,
        &format!("{} ({} sections toggled)", section_hash, count),
    )?;
    Ok(count)
}

pub fn revoke_by_author(ob_dir: &Path, author_name: &str, reverse: bool) -> Result<usize> {
    let authors = crate::authors::query(ob_dir, Some(author_name), None)?;
    if authors.is_empty() {
        return Ok(0);
    }
    let mut count = 0;
    for author in &authors {
        let bucket = &author.id[..2];
        let shard_path = ob_dir.join(".ob").join("authors").join(bucket);
        if !shard_path.exists() {
            continue;
        }
        let records = crate::storage::jsonl_read(&shard_path)?;
        let mut updated = Vec::new();
        for mut rec in records {
            if rec.get("id").and_then(|v| v.as_str()) == Some(&author.id) {
                let revoked = rec
                    .get("revoked")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                rec.as_object_mut()
                    .map(|m| m.insert("revoked".to_string(), serde_json::Value::Bool(!revoked)));
                count += 1;
            }
            updated.push(rec);
        }
        let mut file = std::fs::File::create(&shard_path)?;
        for rec in &updated {
            writeln!(file, "{}", serde_json::to_string(rec)?)?;
        }
    }
    let op_name = if reverse {
        "revoke_reverse_author"
    } else {
        "revoke_author"
    };
    crate::oplog::append(
        ob_dir,
        op_name,
        &format!("{} ({} authors toggled)", author_name, count),
    )?;
    Ok(count)
}

pub fn revoke_by_author_token(
    ob_dir: &Path,
    author_name: &str,
    tokenizer: &str,
    reverse: bool,
) -> Result<usize> {
    let authors = crate::authors::query(ob_dir, Some(author_name), None)?;
    if authors.is_empty() {
        return Ok(0);
    }
    let author_ids: HashSet<String> = authors.iter().map(|a| a.id.clone()).collect();

    let section_dir = ob_dir.join(".ob").join("sections");
    let mut section_hashes: HashSet<String> = HashSet::new();
    if section_dir.is_dir() {
        for entry in std::fs::read_dir(&section_dir)? {
            let entry = entry?;
            if !entry.path().is_file() {
                continue;
            }
            let records = crate::storage::jsonl_read(&entry.path())?;
            for rec in &records {
                if let Some(authors_arr) = rec.get("authors").and_then(|v| v.as_array()) {
                    let has_overlap = authors_arr.iter().any(|a| {
                        a.as_str()
                            .map(|s| author_ids.contains(s))
                            .unwrap_or(false)
                    });
                    if has_overlap {
                        if let Some(hash) = rec.get("section_hash").and_then(|v| v.as_str()) {
                            section_hashes.insert(hash.to_string());
                        }
                    }
                }
            }
        }
    }

    if section_hashes.is_empty() {
        return Ok(0);
    }

    let count = if let Ok(Some(bin_idx)) =
        crate::token_bin_index::TokenBinIndex::open(ob_dir, tokenizer)
    {
        if bin_idx.is_fresh() {
            let mut source_hash_bytes: HashSet<[u8; 32]> = HashSet::new();
            for sh in &section_hashes {
                if let Ok(bytes) = hex::decode(sh) {
                    if let Ok(arr) = bytes.try_into() {
                        source_hash_bytes.insert(arr);
                    }
                }
            }

            let matching_indices = bin_idx.lookup_by_sources(&source_hash_bytes);
            let mut batch: Vec<(u32, bool)> = Vec::with_capacity(matching_indices.len());
            for &idx in &matching_indices {
                let eref = match bin_idx.get_entry_ref(idx) {
                    Some(e) => e,
                    None => continue,
                };
                batch.push((idx, !eref.revoked));
            }
            let toggled = batch.len();
            bin_idx.set_revoked_batch(&batch)?;
            toggled
        } else {
            crate::token_index::revoke_by_sources_fast(ob_dir, tokenizer, &section_hashes)?
        }
    } else {
        crate::token_index::revoke_by_sources_fast(ob_dir, tokenizer, &section_hashes)?
    };

    let op_name = if reverse {
        "revoke_reverse_token"
    } else {
        "revoke_token"
    };
    crate::oplog::append(
        ob_dir,
        op_name,
        &format!("{} tokenizer={} count={}", author_name, tokenizer, count),
    )?;
    Ok(count)
}

pub fn revoke_manifest(ob_dir: &Path, line_hash: &str, file: &str, reverse: bool) -> Result<usize> {
    let bucket = &line_hash[..2];
    let shard_path = ob_dir.join(".ob").join("document-index").join(bucket);
    if !shard_path.exists() {
        return Ok(0);
    }
    let records = crate::storage::jsonl_read(&shard_path)?;
    let mut count = 0;
    let mut updated = Vec::new();
    for mut rec in records {
        let matches = rec.get("line_hash").and_then(|v| v.as_str()) == Some(line_hash)
            && rec.get("file").and_then(|v| v.as_str()) == Some(file);
        if matches {
            let revoked = rec
                .get("revoked")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            rec.as_object_mut()
                .map(|m| m.insert("revoked".to_string(), serde_json::Value::Bool(!revoked)));
            count += 1;
        }
        updated.push(rec);
    }
    let mut f = std::fs::File::create(&shard_path)?;
    for rec in &updated {
        writeln!(f, "{}", serde_json::to_string(rec)?)?;
    }
    let op_name = if reverse {
        "revoke_reverse_manifest"
    } else {
        "revoke_manifest"
    };
    crate::oplog::append(
        ob_dir,
        op_name,
        &format!(
            "line_hash={} file={} ({} toggled)",
            &line_hash[..line_hash.len().min(12)],
            file,
            count
        ),
    )?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_ob_dir(tmp: &TempDir) -> std::path::PathBuf {
        let ob = tmp.path().join(".ob");
        std::fs::create_dir_all(&ob).unwrap();
        tmp.path().to_path_buf()
    }

    fn write_merged_token_file(
        ob_dir: &Path,
        tokenizer: &str,
        file_num: u64,
        entries: &[crate::token_index::TokenIndexEntry],
    ) {
        let dir = ob_dir
            .join(".ob")
            .join(format!("token-index.{}", tokenizer));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{:03}", file_num));
        let mut file = std::fs::File::create(&path).unwrap();
        for e in entries {
            writeln!(file, "{}", serde_json::to_string(e).unwrap()).unwrap();
        }
    }

    #[test]
    fn test_revoke_by_author_token() {
        let tmp = TempDir::new().unwrap();
        let ob_dir = make_ob_dir(&tmp);
        std::fs::create_dir_all(ob_dir.join(".ob").join("authors")).unwrap();
        std::fs::create_dir_all(ob_dir.join(".ob").join("sections")).unwrap();

        let author =
            crate::authors::add(&ob_dir, "Alice", "alice@example.com").unwrap();

        let section = crate::register::register_section(
            &ob_dir,
            "src/main.rs",
            &[author.id.clone()],
            &[],
            "MIT",
            "2024",
        )
        .unwrap();

        let matching_entry = crate::token_index::TokenIndexEntry {
            token_count: 10,
            sources: vec![section.section_hash.clone()],
            tokenizer: "gpt2".to_string(),
            revoked: false,
        };
        let non_matching_entry = crate::token_index::TokenIndexEntry {
            token_count: 20,
            sources: vec!["deadbeef".to_string()],
            tokenizer: "gpt2".to_string(),
            revoked: false,
        };
        write_merged_token_file(&ob_dir, "gpt2", 0, &[matching_entry, non_matching_entry]);

        let count = revoke_by_author_token(&ob_dir, "Alice", "gpt2", false).unwrap();
        assert_eq!(count, 1);

        let entries = crate::token_index::read_merged(&ob_dir, "gpt2").unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries[0].revoked);
        assert!(!entries[1].revoked);

        let count2 = revoke_by_author_token(&ob_dir, "Alice", "gpt2", true).unwrap();
        assert_eq!(count2, 1);

        let entries2 = crate::token_index::read_merged(&ob_dir, "gpt2").unwrap();
        assert!(!entries2[0].revoked);
        assert!(!entries2[1].revoked);
    }

    #[test]
    fn test_revoke_section() {
        let tmp = TempDir::new().unwrap();
        let ob_dir = make_ob_dir(&tmp);
        std::fs::create_dir_all(ob_dir.join(".ob").join("authors")).unwrap();
        std::fs::create_dir_all(ob_dir.join(".ob").join("sections")).unwrap();

        let author =
            crate::authors::add(&ob_dir, "Alice", "alice@example.com").unwrap();

        let section = crate::register::register_section(
            &ob_dir,
            "src/main.rs",
            &[author.id.clone()],
            &[],
            "MIT",
            "2024",
        )
        .unwrap();

        let count = revoke_section(&ob_dir, &section.section_hash, false).unwrap();
        assert_eq!(count, 1);

        let bucket = &section.section_hash[..2];
        let shard_path = ob_dir.join(".ob").join("sections").join(bucket);
        let records = crate::storage::jsonl_read(&shard_path).unwrap();
        let rec = records.iter().find(|r| r.get("section_hash").and_then(|v| v.as_str()) == Some(&section.section_hash)).unwrap();
        assert_eq!(rec.get("revoked").and_then(|v| v.as_bool()), Some(true));
    }

    #[test]
    fn test_revoke_section_reverse() {
        let tmp = TempDir::new().unwrap();
        let ob_dir = make_ob_dir(&tmp);
        std::fs::create_dir_all(ob_dir.join(".ob").join("authors")).unwrap();
        std::fs::create_dir_all(ob_dir.join(".ob").join("sections")).unwrap();

        let author =
            crate::authors::add(&ob_dir, "Alice", "alice@example.com").unwrap();

        let section = crate::register::register_section(
            &ob_dir,
            "src/main.rs",
            &[author.id.clone()],
            &[],
            "MIT",
            "2024",
        )
        .unwrap();

        revoke_section(&ob_dir, &section.section_hash, false).unwrap();
        let count = revoke_section(&ob_dir, &section.section_hash, true).unwrap();
        assert_eq!(count, 1);

        let bucket = &section.section_hash[..2];
        let shard_path = ob_dir.join(".ob").join("sections").join(bucket);
        let records = crate::storage::jsonl_read(&shard_path).unwrap();
        let rec = records.iter().find(|r| r.get("section_hash").and_then(|v| v.as_str()) == Some(&section.section_hash)).unwrap();
        assert_eq!(rec.get("revoked").and_then(|v| v.as_bool()), Some(false));
    }

    #[test]
    fn test_revoke_section_nonexistent() {
        let tmp = TempDir::new().unwrap();
        let ob_dir = make_ob_dir(&tmp);
        std::fs::create_dir_all(ob_dir.join(".ob").join("sections")).unwrap();

        let count = revoke_section(&ob_dir, "deadbeef00", false).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_revoke_manifest() {
        let tmp = TempDir::new().unwrap();
        let ob_dir = make_ob_dir(&tmp);
        std::fs::create_dir_all(ob_dir.join(".ob").join("document-index")).unwrap();

        crate::indexer::index_document(
            &ob_dir,
            "aaaa1111111111aaaa1111111111aaaa1111111111aaaa1111",
            "data.jsonl",
            &["sec1".to_string()],
            "track",
        )
        .unwrap();

        let count = revoke_manifest(
            &ob_dir,
            "aaaa1111111111aaaa1111111111aaaa1111111111aaaa1111",
            "data.jsonl",
            false,
        )
        .unwrap();
        assert_eq!(count, 1);

        let bucket = &"aaaa1111111111aaaa1111111111aaaa1111111111aaaa1111"[..2];
        let shard_path = ob_dir.join(".ob").join("document-index").join(bucket);
        let records = crate::storage::jsonl_read(&shard_path).unwrap();
        assert_eq!(records.len(), 1);
        assert!(records[0].get("revoked").and_then(|v| v.as_bool()).unwrap());

        let count2 = revoke_manifest(
            &ob_dir,
            "aaaa1111111111aaaa1111111111aaaa1111111111aaaa1111",
            "data.jsonl",
            true,
        )
        .unwrap();
        assert_eq!(count2, 1);

        let records2 = crate::storage::jsonl_read(&shard_path).unwrap();
        assert!(!records2[0].get("revoked").and_then(|v| v.as_bool()).unwrap());
    }

    #[test]
    fn test_revoke_manifest_nonexistent() {
        let tmp = TempDir::new().unwrap();
        let ob_dir = make_ob_dir(&tmp);
        std::fs::create_dir_all(ob_dir.join(".ob").join("document-index")).unwrap();

        let count =
            revoke_manifest(&ob_dir, "deadbeef00", "data.jsonl", false).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_revoke_manifest_multiple_in_shard() {
        let tmp = TempDir::new().unwrap();
        let ob_dir = make_ob_dir(&tmp);
        std::fs::create_dir_all(ob_dir.join(".ob").join("document-index")).unwrap();

        crate::indexer::index_document(
            &ob_dir,
            "aaaa1111111111aaaa1111111111aaaa1111111111aaaa1111",
            "data.jsonl",
            &["sec1".to_string()],
            "track",
        )
        .unwrap();
        crate::indexer::index_document(
            &ob_dir,
            "aaaa2222222222aaaa2222222222aaaa2222222222aaaa2222",
            "data.jsonl",
            &["sec2".to_string()],
            "track",
        )
        .unwrap();

        let count = revoke_manifest(
            &ob_dir,
            "aaaa1111111111aaaa1111111111aaaa1111111111aaaa1111",
            "data.jsonl",
            false,
        )
        .unwrap();
        assert_eq!(count, 1);

        let bucket = "aa";
        let shard_path = ob_dir.join(".ob").join("document-index").join(bucket);
        let records = crate::storage::jsonl_read(&shard_path).unwrap();
        assert_eq!(records.len(), 2);

        let first = records
            .iter()
            .find(|r| {
                r.get("line_hash").and_then(|v| v.as_str())
                    == Some("aaaa1111111111aaaa1111111111aaaa1111111111aaaa1111")
            })
            .unwrap();
        assert!(first.get("revoked").and_then(|v| v.as_bool()).unwrap());

        let second = records
            .iter()
            .find(|r| {
                r.get("line_hash").and_then(|v| v.as_str())
                    == Some("aaaa2222222222aaaa2222222222aaaa2222222222aaaa2222")
            })
            .unwrap();
        assert!(!second.get("revoked").and_then(|v| v.as_bool()).unwrap());
    }
}
