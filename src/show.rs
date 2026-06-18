use anyhow::Result;
use rayon::prelude::*;
use std::collections::HashSet;
use std::path::Path;

pub fn show_by_author(ob_dir: &Path, author_name: &str) -> Result<Vec<crate::indexer::DocumentRecord>> {
    if crate::binary_index::BinaryIndex::open(ob_dir)?.is_some() {
        return show_by_author_indexed(ob_dir, author_name);
    }

    let authors = crate::authors::query(ob_dir, Some(author_name), None)?;
    if authors.is_empty() {
        return Ok(Vec::new());
    }
    let author_ids: Vec<&str> = authors.iter().map(|a| a.id.as_str()).collect();
    let sections = find_author_sections_parallel(ob_dir, &author_ids)?;
    find_documents_for_sections_parallel(ob_dir, &sections)
}

pub fn show_by_author_indexed(
    ob_dir: &Path,
    author_name: &str,
) -> Result<Vec<crate::indexer::DocumentRecord>> {
    let authors = crate::authors::query(ob_dir, Some(author_name), None)?;
    if authors.is_empty() {
        return Ok(Vec::new());
    }

    let author_ids: HashSet<String> = authors.iter().map(|a| a.id.clone()).collect();
    let author_id_refs: HashSet<&str> = author_ids.iter().map(|s| s.as_str()).collect();

    let bin_idx = match crate::binary_index::BinaryIndex::open(ob_dir)? {
        Some(idx) => idx,
        None => return Err(anyhow::anyhow!("binary index not found")),
    };

    let section_bucket_prefixes: HashSet<String> = author_ids
        .iter()
        .flat_map(|aid| bin_idx.lookup(aid))
        .filter_map(|r| match r {
            crate::binary_index::IndexRef::DocumentShard { prefix } => {
                Some(format!("{:02x}", prefix))
            }
            _ => None,
        })
        .collect();

    let mut section_hashes: HashSet<String> = HashSet::new();
    for prefix in &section_bucket_prefixes {
        let path = ob_dir.join(".ob").join("sections").join(prefix);
        let mmap = match crate::mmap_lines::mmap_file(&path)? {
            Some(m) => m,
            None => continue,
        };
        for line in crate::mmap_lines::iter_lines(&mmap) {
            if let Ok(section) = serde_json::from_slice::<crate::register::Section>(line) {
                if section
                    .authors
                    .iter()
                    .any(|a| author_id_refs.contains(a.as_str()))
                {
                    section_hashes.insert(section.section_hash);
                }
            }
        }
    }

    let manifest_bucket_prefixes: HashSet<String> = section_hashes
        .iter()
        .flat_map(|sh| bin_idx.lookup(sh))
        .filter_map(|r| match r {
            crate::binary_index::IndexRef::DocumentShard { prefix } => {
                Some(format!("{:02x}", prefix))
            }
            _ => None,
        })
        .collect();

    let section_hash_refs: HashSet<&str> = section_hashes.iter().map(|s| s.as_str()).collect();
    let mut results = Vec::new();
    for prefix in &manifest_bucket_prefixes {
        let path = ob_dir.join(".ob").join("document-index").join(prefix);
        let mmap = match crate::mmap_lines::mmap_file(&path)? {
            Some(m) => m,
            None => continue,
        };
        for line in crate::mmap_lines::iter_lines(&mmap) {
            if let Ok(m) = serde_json::from_slice::<crate::indexer::DocumentRecord>(line) {
                if m.sources
                    .iter()
                    .any(|s| section_hash_refs.contains(s.as_str()))
                {
                    results.push(m);
                }
            }
        }
    }

    Ok(results)
}

pub struct TokenShowResult {
    pub total_tokens: u64,
    pub document_count: usize,
    pub entries: Vec<crate::token_index::TokenIndexEntry>,
}

pub fn show_by_author_token(
    ob_dir: &Path,
    author_name: &str,
    tokenizer: &str,
) -> Result<TokenShowResult> {
    let authors = crate::authors::query(ob_dir, Some(author_name), None)?;
    if authors.is_empty() {
        return Ok(TokenShowResult {
            total_tokens: 0,
            document_count: 0,
            entries: Vec::new(),
        });
    }

    let author_ids: Vec<&str> = authors.iter().map(|a| a.id.as_str()).collect();
    let sections = find_author_sections_parallel(ob_dir, &author_ids)?;
    let section_hashes: HashSet<&str> = sections.iter().map(|s| s.as_str()).collect();

    if let Ok(Some(bin_idx)) = crate::token_bin_index::TokenBinIndex::open(ob_dir, tokenizer) {
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

            let mut merged_dir_map: std::collections::HashMap<u16, memmap2::Mmap> =
                std::collections::HashMap::new();
            for &idx in &matching_indices {
                let eref = match bin_idx.get_entry_ref(idx) {
                    Some(e) => e,
                    None => continue,
                };
                if eref.revoked {
                    continue;
                }
                if !merged_dir_map.contains_key(&eref.jsonl_file_number) {
                    let path = ob_dir
                        .join(".ob")
                        .join(format!("token-index.{}", tokenizer))
                        .join(format!("{:03}", eref.jsonl_file_number));
                    if let Some(mmap) = crate::mmap_lines::mmap_file(&path)? {
                        merged_dir_map.insert(eref.jsonl_file_number, mmap);
                    }
                }
            }

            let mut entries: Vec<crate::token_index::TokenIndexEntry> = Vec::new();
            for &idx in &matching_indices {
                let eref = match bin_idx.get_entry_ref(idx) {
                    Some(e) => e,
                    None => continue,
                };
                if eref.revoked {
                    continue;
                }
                let mmap = match merged_dir_map.get(&eref.jsonl_file_number) {
                    Some(m) => m,
                    None => continue,
                };
                let start = eref.jsonl_byte_offset as usize;
                let end = start + eref.jsonl_length as usize;
                if end > mmap.len() {
                    continue;
                }
                if let Ok(e) = serde_json::from_slice::<crate::token_index::TokenIndexEntry>(
                    &mmap[start..end],
                ) {
                    entries.push(e);
                }
            }

            let total_tokens = entries.iter().map(|e| e.token_count).sum();
            let document_count = entries.len();

            return Ok(TokenShowResult {
                total_tokens,
                document_count,
                entries,
            });
        }
    }

    let source_hash_owned: HashSet<String> =
        section_hashes.iter().map(|s| s.to_string()).collect();
    let entries: Vec<crate::token_index::TokenIndexEntry> = crate::token_index::find_entries_by_sources_fast(ob_dir, tokenizer, &source_hash_owned)?
        .into_iter()
        .filter(|e| !e.revoked)
        .collect();

    let total_tokens = entries.iter().map(|e| e.token_count).sum();
    let document_count = entries.len();

    Ok(TokenShowResult {
        total_tokens,
        document_count,
        entries,
    })
}

pub fn show_by_section_token(
    ob_dir: &Path,
    section_hash: &str,
    tokenizer: &str,
) -> Result<TokenShowResult> {
    // section_hash is used directly — no author→section resolution needed.
    let mut section_hashes: HashSet<String> = HashSet::new();
    section_hashes.insert(section_hash.to_string());

    if let Ok(Some(bin_idx)) = crate::token_bin_index::TokenBinIndex::open(ob_dir, tokenizer) {
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

            let mut merged_dir_map: std::collections::HashMap<u16, memmap2::Mmap> =
                std::collections::HashMap::new();
            for &idx in &matching_indices {
                let eref = match bin_idx.get_entry_ref(idx) {
                    Some(e) => e,
                    None => continue,
                };
                if eref.revoked {
                    continue;
                }
                if !merged_dir_map.contains_key(&eref.jsonl_file_number) {
                    let path = ob_dir
                        .join(".ob")
                        .join(format!("token-index.{}", tokenizer))
                        .join(format!("{:03}", eref.jsonl_file_number));
                    if let Some(mmap) = crate::mmap_lines::mmap_file(&path)? {
                        merged_dir_map.insert(eref.jsonl_file_number, mmap);
                    }
                }
            }

            let mut entries: Vec<crate::token_index::TokenIndexEntry> = Vec::new();
            for &idx in &matching_indices {
                let eref = match bin_idx.get_entry_ref(idx) {
                    Some(e) => e,
                    None => continue,
                };
                if eref.revoked {
                    continue;
                }
                let mmap = match merged_dir_map.get(&eref.jsonl_file_number) {
                    Some(m) => m,
                    None => continue,
                };
                let start = eref.jsonl_byte_offset as usize;
                let end = start + eref.jsonl_length as usize;
                if end > mmap.len() {
                    continue;
                }
                if let Ok(e) = serde_json::from_slice::<crate::token_index::TokenIndexEntry>(
                    &mmap[start..end],
                ) {
                    entries.push(e);
                }
            }

            let total_tokens = entries.iter().map(|e| e.token_count).sum();
            let document_count = entries.len();
            return Ok(TokenShowResult {
                total_tokens,
                document_count,
                entries,
            });
        }
    }

    let entries: Vec<crate::token_index::TokenIndexEntry> =
        crate::token_index::find_entries_by_sources_fast(ob_dir, tokenizer, &section_hashes)?
            .into_iter()
            .filter(|e| !e.revoked)
            .collect();

    let total_tokens = entries.iter().map(|e| e.token_count).sum();
    let document_count = entries.len();
    Ok(TokenShowResult {
        total_tokens,
        document_count,
        entries,
    })
}

fn find_author_sections_parallel(ob_dir: &Path, author_ids: &[&str]) -> Result<Vec<String>> {
    let section_dir = ob_dir.join(".ob").join("sections");
    if !section_dir.is_dir() {
        return Ok(Vec::new());
    }

    let entries: Vec<std::fs::DirEntry> = std::fs::read_dir(&section_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .collect();

    let author_id_set: HashSet<&str> = author_ids.iter().copied().collect();

    let results: Vec<String> = entries
        .par_iter()
        .flat_map(|entry| {
            let mmap = match crate::mmap_lines::mmap_file(&entry.path()) {
                Ok(Some(m)) => m,
                _ => return Vec::new(),
            };
            crate::mmap_lines::iter_lines(&mmap)
                .filter_map(|line| {
                    let section: crate::register::Section = serde_json::from_slice(line).ok()?;
                    if section
                        .authors
                        .iter()
                        .any(|a| author_id_set.contains(a.as_str()))
                    {
                        Some(section.section_hash)
                    } else {
                        None
                    }
                })
                .collect()
        })
        .collect();

    Ok(results)
}

fn find_documents_for_sections_parallel(
    ob_dir: &Path,
    section_hashes: &[String],
) -> Result<Vec<crate::indexer::DocumentRecord>> {
    let manifest_dir = ob_dir.join(".ob").join("document-index");
    if !manifest_dir.is_dir() {
        return Ok(Vec::new());
    }

    let entries: Vec<std::fs::DirEntry> = std::fs::read_dir(&manifest_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .collect();

    let section_set: HashSet<&str> = section_hashes.iter().map(|s| s.as_str()).collect();

    let results: Vec<crate::indexer::DocumentRecord> = entries
        .par_iter()
        .flat_map(|entry| {
            let mmap = match crate::mmap_lines::mmap_file(&entry.path()) {
                Ok(Some(m)) => m,
                _ => return Vec::new(),
            };
            crate::mmap_lines::iter_lines(&mmap)
                .filter_map(|line| {
                    let m: crate::indexer::DocumentRecord = serde_json::from_slice(line).ok()?;
                    if m.sources.iter().any(|s| section_set.contains(s.as_str())) {
                        Some(m)
                    } else {
                        None
                    }
                })
                .collect()
        })
        .collect();

    Ok(results)
}

// ---------------------------------------------------------------------------
// --section HASH: filter manifests by section hash
// ---------------------------------------------------------------------------

pub fn show_by_section(ob_dir: &Path, section_hash: &str) -> Result<Vec<crate::indexer::DocumentRecord>> {
    find_documents_for_sections_parallel(ob_dir, &[section_hash.to_string()])
}

// ---------------------------------------------------------------------------
// --license NAME: filter sections by license, then find manifests
// ---------------------------------------------------------------------------

pub fn show_by_license(ob_dir: &Path, license: &str) -> Result<Vec<crate::indexer::DocumentRecord>> {
    let sections = find_license_sections_parallel(ob_dir, license)?;
    if sections.is_empty() {
        return Ok(Vec::new());
    }
    find_documents_for_sections_parallel(ob_dir, &sections)
}

fn find_license_sections_parallel(ob_dir: &Path, license: &str) -> Result<Vec<String>> {
    let section_dir = ob_dir.join(".ob").join("sections");
    if !section_dir.is_dir() {
        return Ok(Vec::new());
    }

    let entries: Vec<std::fs::DirEntry> = std::fs::read_dir(&section_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .collect();

    let results: Vec<String> = entries
        .par_iter()
        .flat_map(|entry| {
            let mmap = match crate::mmap_lines::mmap_file(&entry.path()) {
                Ok(Some(m)) => m,
                _ => return Vec::new(),
            };
            crate::mmap_lines::iter_lines(&mmap)
                .filter_map(|line| {
                    let section: crate::register::Section = serde_json::from_slice(line).ok()?;
                    if section.license == license {
                        Some(section.section_hash)
                    } else {
                        None
                    }
                })
                .collect()
        })
        .collect();

    Ok(results)
}

// ---------------------------------------------------------------------------
// --file PATH: filter manifests by file path
// ---------------------------------------------------------------------------

pub fn filter_documents_by_file(
    manifests: Vec<crate::indexer::DocumentRecord>,
    file_path: &str,
) -> Vec<crate::indexer::DocumentRecord> {
    manifests.into_iter().filter(|m| m.file == file_path).collect()
}

// ---------------------------------------------------------------------------
// --revoked: filter to only revoked entries
// ---------------------------------------------------------------------------

/// For manifests: find sections belonging to revoked authors, then filter.
pub fn filter_documents_revoked(
    ob_dir: &Path,
    manifests: Vec<crate::indexer::DocumentRecord>,
) -> Result<Vec<crate::indexer::DocumentRecord>> {
    let revoked_section_hashes = find_revoked_sections(ob_dir)?;
    if revoked_section_hashes.is_empty() {
        return Ok(Vec::new());
    }
    let set: HashSet<&str> = revoked_section_hashes.iter().map(|s| s.as_str()).collect();
    Ok(manifests
        .into_iter()
        .filter(|m| m.sources.iter().any(|s| set.contains(s.as_str())))
        .collect())
}

/// Filter to only active (non-revoked) manifests.
pub fn filter_documents_active(
    ob_dir: &Path,
    manifests: Vec<crate::indexer::DocumentRecord>,
) -> Result<Vec<crate::indexer::DocumentRecord>> {
    let revoked_section_hashes = find_revoked_sections(ob_dir)?;
    if revoked_section_hashes.is_empty() {
        return Ok(manifests);
    }
    let set: HashSet<&str> = revoked_section_hashes.iter().map(|s| s.as_str()).collect();
    Ok(manifests
        .into_iter()
        .filter(|m| !m.sources.iter().any(|s| set.contains(s.as_str())))
        .collect())
}

/// For token-level: filter to only revoked entries.
pub fn filter_token_entries_revoked(
    entries: Vec<crate::token_index::TokenIndexEntry>,
) -> Vec<crate::token_index::TokenIndexEntry> {
    entries.into_iter().filter(|e| e.revoked).collect()
}

fn find_revoked_sections(ob_dir: &Path) -> Result<Vec<String>> {
    // 1. Find all revoked authors
    let authors_dir = ob_dir.join(".ob").join("authors");
    if !authors_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut revoked_author_ids: HashSet<String> = HashSet::new();
    for entry in std::fs::read_dir(&authors_dir)?.filter_map(|e| e.ok()) {
        if !entry.path().is_file() {
            continue;
        }
        let mmap = match crate::mmap_lines::mmap_file(&entry.path())? {
            Some(m) => m,
            None => continue,
        };
        for line in crate::mmap_lines::iter_lines(&mmap) {
            if let Ok(val) = serde_json::from_slice::<serde_json::Value>(line) {
                let is_revoked = val
                    .get("revoked")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if is_revoked {
                    if let Some(id) = val.get("id").and_then(|v| v.as_str()) {
                        revoked_author_ids.insert(id.to_string());
                    }
                }
            }
        }
    }

    // 2. Find sections whose authors include any revoked author OR sections directly revoked
    let section_dir = ob_dir.join(".ob").join("sections");
    if !section_dir.is_dir() {
        return Ok(Vec::new());
    }

    let id_refs: HashSet<&str> = revoked_author_ids.iter().map(|s| s.as_str()).collect();
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
                let directly_revoked = val
                    .get("revoked")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if directly_revoked {
                    if let Some(sh) = val.get("section_hash").and_then(|v| v.as_str()) {
                        results.push(sh.to_string());
                        continue;
                    }
                }
                if !revoked_author_ids.is_empty() {
                    if let Some(authors_arr) = val.get("authors").and_then(|v| v.as_array()) {
                        let has_revoked = authors_arr
                            .iter()
                            .any(|a| a.as_str().map(|s| id_refs.contains(s)).unwrap_or(false));
                        if has_revoked {
                            if let Some(sh) = val.get("section_hash").and_then(|v| v.as_str()) {
                                results.push(sh.to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(results)
}

/// Show all manifests in the document-index (no author/section filter).
pub fn show_all(ob_dir: &Path) -> Result<Vec<crate::indexer::DocumentRecord>> {
    let manifest_dir = ob_dir.join(".ob").join("document-index");
    if !manifest_dir.is_dir() {
        return Ok(Vec::new());
    }

    let entries: Vec<std::fs::DirEntry> = std::fs::read_dir(&manifest_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .collect();

    let results: Vec<crate::indexer::DocumentRecord> = entries
        .par_iter()
        .flat_map(|entry| {
            let mmap = match crate::mmap_lines::mmap_file(&entry.path()) {
                Ok(Some(m)) => m,
                _ => return Vec::new(),
            };
            crate::mmap_lines::iter_lines(&mmap)
                .filter_map(|line| serde_json::from_slice::<crate::indexer::DocumentRecord>(line).ok())
                .collect()
        })
        .collect();

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_ob_dir(tmp: &TempDir) -> std::path::PathBuf {
        let ob = tmp.path().join(".ob");
        std::fs::create_dir_all(ob.join("authors")).unwrap();
        std::fs::create_dir_all(ob.join("sections")).unwrap();
        std::fs::create_dir_all(ob.join("document-index")).unwrap();
        tmp.path().to_path_buf()
    }

    fn write_section(ob_dir: &Path, section_hash: &str, author_ids: &[&str]) {
        let section = crate::register::Section {
            section_hash: section_hash.to_string(),
            path: String::new(),
            authors: author_ids.iter().map(|s| s.to_string()).collect(),
            contributors: vec![],
            license: String::new(),
            year: String::new(),
        };
        let path = ob_dir.join(".ob").join("sections").join("00");
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(file, "{}", serde_json::to_string(&section).unwrap()).unwrap();
    }

    fn write_token_merged_file(
        ob_dir: &Path,
        tokenizer: &str,
        file_num: u64,
        entries: &[crate::token_index::TokenIndexEntry],
    ) {
        let dir = ob_dir.join(".ob").join(format!("token-index.{}", tokenizer));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{:03}", file_num));
        use std::io::Write;
        let mut file = std::fs::File::create(&path).unwrap();
        for e in entries {
            writeln!(file, "{}", serde_json::to_string(e).unwrap()).unwrap();
        }
    }

    #[test]
    fn test_show_by_author_token() {
        let tmp = TempDir::new().unwrap();
        let ob_dir = make_ob_dir(&tmp);

        let author = crate::authors::add(&ob_dir, "alice", "alice@example.com").unwrap();
        write_section(&ob_dir, "sh_hash1", &[&author.id]);

        let entries = vec![
            crate::token_index::TokenIndexEntry {
                token_count: 100,
                sources: vec!["sh_hash1".to_string()],
                tokenizer: "gpt2".to_string(),
                revoked: false,
            },
            crate::token_index::TokenIndexEntry {
                token_count: 200,
                sources: vec!["sh_hash1".to_string(), "other".to_string()],
                tokenizer: "gpt2".to_string(),
                revoked: false,
            },
            // Revoked — should be excluded
            crate::token_index::TokenIndexEntry {
                token_count: 999,
                sources: vec!["sh_hash1".to_string()],
                tokenizer: "gpt2".to_string(),
                revoked: true,
            },
            // Different section — should be excluded
            crate::token_index::TokenIndexEntry {
                token_count: 50,
                sources: vec!["unrelated_hash".to_string()],
                tokenizer: "gpt2".to_string(),
                revoked: false,
            },
        ];
        write_token_merged_file(&ob_dir, "gpt2", 0, &entries);

        let result = show_by_author_token(&ob_dir, "alice", "gpt2").unwrap();
        assert_eq!(result.entries.len(), 2);
        assert_eq!(result.total_tokens, 300);
        assert_eq!(result.document_count, 2);
    }

    #[test]
    fn test_show_by_author_token_no_author() {
        let tmp = TempDir::new().unwrap();
        let ob_dir = make_ob_dir(&tmp);

        let result = show_by_author_token(&ob_dir, "nonexistent", "gpt2").unwrap();
        assert_eq!(result.total_tokens, 0);
        assert_eq!(result.document_count, 0);
        assert!(result.entries.is_empty());
    }

    #[test]
    fn test_show_by_author_token_no_matching_sections() {
        let tmp = TempDir::new().unwrap();
        let ob_dir = make_ob_dir(&tmp);

        crate::authors::add(&ob_dir, "bob", "bob@example.com").unwrap();

        let entries = vec![crate::token_index::TokenIndexEntry {
            token_count: 50,
            sources: vec!["some_hash".to_string()],
            tokenizer: "gpt2".to_string(),
            revoked: false,
        }];
        write_token_merged_file(&ob_dir, "gpt2", 0, &entries);

        let result = show_by_author_token(&ob_dir, "bob", "gpt2").unwrap();
        assert_eq!(result.total_tokens, 0);
        assert_eq!(result.document_count, 0);
    }

    fn write_manifest(ob_dir: &Path, line_hash: &str, file: &str, sources: &[&str]) {
        let manifest = crate::indexer::DocumentRecord {
            line_hash: line_hash.to_string(),
            file: file.to_string(),
            sources: sources.iter().map(|s| s.to_string()).collect(),
        };
        let bucket = &line_hash[..2];
        let path = ob_dir.join(".ob").join("document-index").join(bucket);
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(f, "{}", serde_json::to_string(&manifest).unwrap()).unwrap();
    }

    fn write_section_with_license(ob_dir: &Path, section_hash: &str, author_ids: &[&str], license: &str) {
        let section = crate::register::Section {
            section_hash: section_hash.to_string(),
            path: String::new(),
            authors: author_ids.iter().map(|s| s.to_string()).collect(),
            contributors: vec![],
            license: license.to_string(),
            year: String::new(),
        };
        let path = ob_dir.join(".ob").join("sections").join("00");
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(file, "{}", serde_json::to_string(&section).unwrap()).unwrap();
    }

    fn write_author_revoked(ob_dir: &Path, name: &str, email: &str, revoked: bool) -> String {
        let id = crate::hash::compute_hash(&serde_json::json!({"name": name, "email": email}));
        let bucket = &id[..2];
        let path = ob_dir.join(".ob").join("authors").join(bucket);
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .unwrap();
        let record = serde_json::json!({
            "id": id,
            "name": name,
            "email": email,
            "revoked": revoked,
        });
        writeln!(f, "{}", serde_json::to_string(&record).unwrap()).unwrap();
        id
    }

    #[test]
    fn test_show_by_section() {
        let tmp = TempDir::new().unwrap();
        let ob_dir = make_ob_dir(&tmp);

        write_manifest(&ob_dir, "aaaa1111111111", "data.jsonl", &["sec_hash1", "sec_hash2"]);
        write_manifest(&ob_dir, "bbbb2222222222", "data.jsonl", &["sec_hash1"]);
        write_manifest(&ob_dir, "cccc3333333333", "other.jsonl", &["sec_hash3"]);

        let results = show_by_section(&ob_dir, "sec_hash1").unwrap();
        assert_eq!(results.len(), 2);

        let hashes: Vec<&str> = results.iter().map(|m| m.line_hash.as_str()).collect();
        assert!(hashes.contains(&"aaaa1111111111"));
        assert!(hashes.contains(&"bbbb2222222222"));
    }

    #[test]
    fn test_show_by_section_no_match() {
        let tmp = TempDir::new().unwrap();
        let ob_dir = make_ob_dir(&tmp);

        write_manifest(&ob_dir, "aaaa1111111111", "data.jsonl", &["sec_hash1"]);

        let results = show_by_section(&ob_dir, "nonexistent").unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_show_by_license() {
        let tmp = TempDir::new().unwrap();
        let ob_dir = make_ob_dir(&tmp);

        write_section_with_license(&ob_dir, "sec_a", &[], "MIT");
        write_section_with_license(&ob_dir, "sec_b", &[], "Apache-2.0");
        write_manifest(&ob_dir, "aaaa1111111111", "data.jsonl", &["sec_a"]);
        write_manifest(&ob_dir, "bbbb2222222222", "data.jsonl", &["sec_b"]);

        let results = show_by_license(&ob_dir, "MIT").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].line_hash, "aaaa1111111111");
    }

    #[test]
    fn test_show_by_license_no_match() {
        let tmp = TempDir::new().unwrap();
        let ob_dir = make_ob_dir(&tmp);

        let results = show_by_license(&ob_dir, "GPL-3.0").unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_filter_documents_by_file() {
        let manifests = vec![
            crate::indexer::DocumentRecord {
                line_hash: "a".to_string(),
                file: "data.jsonl".to_string(),
                sources: vec!["s1".to_string()],
            },
            crate::indexer::DocumentRecord {
                line_hash: "b".to_string(),
                file: "other.jsonl".to_string(),
                sources: vec!["s2".to_string()],
            },
        ];
        let filtered = filter_documents_by_file(manifests, "data.jsonl");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].line_hash, "a");
    }

    #[test]
    fn test_filter_documents_revoked() {
        let tmp = TempDir::new().unwrap();
        let ob_dir = make_ob_dir(&tmp);

        let rev_id = write_author_revoked(&ob_dir, "revoked_author", "rev@example.com", true);
        let _ok_id = write_author_revoked(&ob_dir, "active_author", "active@example.com", false);

        write_section(&ob_dir, "sec_revoked", &[&rev_id]);
        write_manifest(&ob_dir, "aaaa1111111111", "data.jsonl", &["sec_revoked"]);
        write_manifest(&ob_dir, "bbbb2222222222", "data.jsonl", &["sec_other"]);

        let manifests = vec![
            crate::indexer::DocumentRecord {
                line_hash: "aaaa1111111111".to_string(),
                file: "data.jsonl".to_string(),
                sources: vec!["sec_revoked".to_string()],
            },
            crate::indexer::DocumentRecord {
                line_hash: "bbbb2222222222".to_string(),
                file: "data.jsonl".to_string(),
                sources: vec!["sec_other".to_string()],
            },
        ];

        let filtered = filter_documents_revoked(&ob_dir, manifests).unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].line_hash, "aaaa1111111111");
    }

    #[test]
    fn test_filter_token_entries_revoked() {
        let entries = vec![
            crate::token_index::TokenIndexEntry {
                token_count: 100,
                sources: vec!["s1".to_string()],
                tokenizer: "gpt2".to_string(),
                revoked: false,
            },
            crate::token_index::TokenIndexEntry {
                token_count: 200,
                sources: vec!["s2".to_string()],
                tokenizer: "gpt2".to_string(),
                revoked: true,
            },
        ];
        let filtered = filter_token_entries_revoked(entries);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].token_count, 200);
    }

    #[test]
    fn test_show_all() {
        let tmp = TempDir::new().unwrap();
        let ob_dir = make_ob_dir(&tmp);

        write_manifest(&ob_dir, "aaaa1111111111", "a.jsonl", &["s1"]);
        write_manifest(&ob_dir, "bbbb2222222222", "b.jsonl", &["s2"]);

        let results = show_all(&ob_dir).unwrap();
        assert_eq!(results.len(), 2);
    }
}
