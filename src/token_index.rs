use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenIndexEntry {
    pub token_count: u64,
    pub sources: Vec<String>,
    pub tokenizer: String,
    pub revoked: bool,
}

pub fn write_pid(ob_dir: &Path, tokenizer: &str, token_count: u64, sources: &[String]) -> Result<()> {
    let ob_dot = ob_dir.join(".ob");
    std::fs::create_dir_all(&ob_dot)?;

    let entry = TokenIndexEntry {
        token_count,
        sources: sources.to_vec(),
        tokenizer: tokenizer.to_string(),
        revoked: false,
    };

    let pid = std::process::id();
    let pid_path = ob_dot.join(format!("token-index.{}.{}", tokenizer, pid));

    let line = serde_json::to_string(&entry)?;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&pid_path)?
        .write_all(format!("{}\n", line).as_bytes())?;
    Ok(())
}

pub fn list_pid_files(ob_dir: &Path, tokenizer: &str) -> Result<Vec<PathBuf>> {
    let ob_dot = ob_dir.join(".ob");
    if !ob_dot.is_dir() {
        return Ok(Vec::new());
    }

    let prefix = format!("token-index.{}.", tokenizer);
    let mut pid_files = Vec::new();

    for entry in std::fs::read_dir(&ob_dot)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(suffix) = name.strip_prefix(&prefix) {
            if entry.path().is_file() && !suffix.is_empty() && suffix != "bin" {
                pid_files.push(entry.path());
            }
        }
    }
    Ok(pid_files)
}

pub fn read_merged(ob_dir: &Path, tokenizer: &str) -> Result<Vec<TokenIndexEntry>> {
    let merged_dir = ob_dir.join(".ob").join(format!("token-index.{}", tokenizer));
    if !merged_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut files: Vec<(u64, PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(&merged_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if let Ok(num) = name.parse::<u64>() {
            files.push((num, entry.path()));
        }
    }
    files.sort_by_key(|(num, _)| *num);

    let mut entries = Vec::new();
    for (_, path) in files {
        let mmap = match crate::mmap_lines::mmap_file(&path)? {
            Some(m) => m,
            None => continue,
        };
        for line in crate::mmap_lines::iter_lines(&mmap) {
            if let Ok(e) = serde_json::from_slice::<TokenIndexEntry>(line) {
                entries.push(e);
            }
        }
    }
    Ok(entries)
}

pub fn revoke_by_sources(
    ob_dir: &Path,
    tokenizer: &str,
    target_sources: &HashSet<String>,
) -> Result<usize> {
    let merged_dir = ob_dir.join(".ob").join(format!("token-index.{}", tokenizer));
    if !merged_dir.is_dir() {
        return Ok(0);
    }

    let mut files: Vec<(u64, PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(&merged_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if let Ok(num) = name.parse::<u64>() {
            files.push((num, entry.path()));
        }
    }
    files.sort_by_key(|(num, _)| *num);

    let mut total_toggled = 0usize;
    for (_, path) in &files {
        let records = crate::storage::jsonl_read(path)?;
        let mut updated = Vec::new();
        let mut toggled_in_file = 0usize;

        for mut rec in records {
            if let Ok(mut entry) = serde_json::from_value::<TokenIndexEntry>(rec.clone()) {
                let matches = entry.sources.iter().any(|s| target_sources.contains(s));
                if matches {
                    entry.revoked = !entry.revoked;
                    toggled_in_file += 1;
                    rec = serde_json::to_value(&entry)?;
                }
            }
            updated.push(rec);
        }

        if toggled_in_file > 0 {
            let mut file = std::fs::File::create(path)?;
            for rec in &updated {
                writeln!(file, "{}", serde_json::to_string(rec)?)?;
            }
            total_toggled += toggled_in_file;
        }
    }
    Ok(total_toggled)
}

pub fn merge_pid_files(ob_dir: &Path, tokenizer: &str) -> Result<(usize, usize)> {
    let pid_files = list_pid_files(ob_dir, tokenizer)?;
    if pid_files.is_empty() {
        return Ok((0, 0));
    }

    let merged_dir = ob_dir
        .join(".ob")
        .join(format!("token-index.{}", tokenizer));
    std::fs::create_dir_all(&merged_dir)?;

    // Determine next file number
    let mut max_num: u64 = 0;
    if merged_dir.is_dir() {
        for entry in std::fs::read_dir(&merged_dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if let Ok(num) = name.parse::<u64>() {
                max_num = max_num.max(num);
            }
        }
        max_num += 1;
    }

    let mut entries_merged = 0usize;
    let mut files_deleted = 0usize;
    let mut next_num = max_num;

    for pid_path in &pid_files {
        let records = crate::storage::jsonl_read(pid_path)?;
        if records.is_empty() {
            let _ = std::fs::remove_file(pid_path);
            files_deleted += 1;
            continue;
        }

        let numbered_path = merged_dir.join(format!("{:03}", next_num));
        let mut file = std::fs::File::create(&numbered_path)?;
        for rec in &records {
            writeln!(file, "{}", serde_json::to_string(rec)?)?;
            entries_merged += 1;
        }
        next_num += 1;

        std::fs::remove_file(pid_path)?;
        files_deleted += 1;
    }

    Ok((entries_merged, files_deleted))
}

pub fn archive_revoked(ob_dir: &Path, tokenizer: &str) -> Result<usize> {
    let merged_dir = ob_dir.join(".ob").join(format!("token-index.{}", tokenizer));
    if !merged_dir.is_dir() {
        return Ok(0);
    }

    let archive_dir = ob_dir.join(".ob").join("archive");
    std::fs::create_dir_all(&archive_dir)?;

    let archive_path = archive_dir.join(format!("token-index.{}.00", tokenizer));

    let mut files: Vec<(u64, PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(&merged_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if let Ok(num) = name.parse::<u64>() {
            files.push((num, entry.path()));
        }
    }
    files.sort_by_key(|(num, _)| *num);

    let mut archived_count = 0usize;
    for (_, path) in &files {
        let records = crate::storage::jsonl_read(path)?;
        let mut kept = Vec::new();
        let mut revoked_in_file = 0usize;

        for rec in &records {
            let is_revoked = rec
                .get("revoked")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if is_revoked {
                crate::storage::jsonl_append(&archive_path, rec)?;
                revoked_in_file += 1;
            } else {
                kept.push(rec.clone());
            }
        }

        if revoked_in_file > 0 {
            let mut file = std::fs::File::create(path)?;
            for rec in &kept {
                writeln!(file, "{}", serde_json::to_string(&rec)?)?;
            }
            archived_count += revoked_in_file;
        }
    }

    Ok(archived_count)
}

pub fn list_tokenizers(ob_dir: &Path) -> Result<Vec<String>> {
    let ob_dot = ob_dir.join(".ob");
    if !ob_dot.is_dir() {
        return Ok(Vec::new());
    }

    let mut tokenizers = HashSet::new();
    let prefix = "token-index.";

    for entry in std::fs::read_dir(&ob_dot)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(suffix) = name.strip_prefix(prefix) {
            if entry.path().is_dir() {
                if !suffix.is_empty() {
                    tokenizers.insert(suffix.to_string());
                }
            }
            if entry.path().is_file() {
                if let Some(dot_pos) = suffix.rfind('.') {
                    let tok_name = &suffix[..dot_pos];
                    if !tok_name.is_empty() {
                        tokenizers.insert(tok_name.to_string());
                    }
                }
            }
        }
    }

    let mut result: Vec<String> = tokenizers.into_iter().collect();
    result.sort();
    Ok(result)
}

#[derive(Debug)]
pub struct TokenStatus {
    pub total_entries: usize,
    pub active_entries: usize,
    pub revoked_entries: usize,
    pub total_tokens: u64,
}

pub fn build_binary_index(ob_dir: &Path, tokenizer: &str) -> Result<crate::token_bin_index::BuildStats> {
    let merged_dir = ob_dir.join(".ob").join(format!("token-index.{}", tokenizer));
    if !merged_dir.is_dir() {
        return Err(anyhow::anyhow!(
            "no merged token-index directory for tokenizer '{}'",
            tokenizer
        ));
    }

    let mut files: Vec<(u64, PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(&merged_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if let Ok(num) = name.parse::<u64>() {
            files.push((num, entry.path()));
        }
    }
    files.sort_by_key(|(num, _)| *num);

    let mut all_entries: Vec<TokenIndexEntry> = Vec::new();
    let mut all_offsets: Vec<(u16, u32, u32)> = Vec::new();

    for (file_number, path) in &files {
        let mmap = match crate::mmap_lines::mmap_file(path)? {
            Some(m) => m,
            None => continue,
        };
        let data = &*mmap;
        let mut pos: usize = 0;
        while pos < data.len() {
            while pos < data.len() && data[pos] == b'\n' {
                pos += 1;
            }
            if pos >= data.len() {
                break;
            }
            let line_start = pos;
            while pos < data.len() && data[pos] != b'\n' {
                pos += 1;
            }
            let line_end = pos;
            let line = &data[line_start..line_end];
            if line.is_empty() {
                continue;
            }
            if let Ok(entry) = serde_json::from_slice::<TokenIndexEntry>(line) {
                let byte_offset = line_start as u32;
                let byte_len = (line_end - line_start) as u32;
                all_entries.push(entry);
                all_offsets.push((*file_number as u16, byte_offset, byte_len));
            }
        }
    }

    crate::token_bin_index::TokenBinIndexBuilder::build(
        ob_dir,
        tokenizer,
        &all_entries,
        &all_offsets,
    )
}

/// Helper: enumerate sorted merged files for a tokenizer.
fn get_merged_files(ob_dir: &Path, tokenizer: &str) -> Result<Vec<(u64, PathBuf)>> {
    let merged_dir = ob_dir.join(".ob").join(format!("token-index.{}", tokenizer));
    if !merged_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut files: Vec<(u64, PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(&merged_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if let Ok(num) = name.parse::<u64>() {
            files.push((num, entry.path()));
        }
    }
    files.sort_by_key(|(num, _)| *num);
    Ok(files)
}

/// Fast multi-needle substring matcher using a 4-byte prefix lookup table.
///
/// For large needle sets (e.g., 2000 hashes), the naive O(needles × line_length)
/// approach is extremely slow. This struct groups needles by their first 4 bytes
/// into a HashMap. When scanning a line, only needles whose 4-byte prefix matches
/// at the current position are considered for full comparison.
///
/// With 2000 needles and a 4-byte prefix, the expected bucket size per quote byte
/// is roughly 2000 / 2^32 ≈ 0, meaning most positions are rejected instantly.
struct FastMatcher {
    /// Maps first 4 bytes of each needle to the list of full needles with that prefix.
    prefix_map: HashMap<[u8; 4], Vec<Vec<u8>>>,
}

impl FastMatcher {
    /// Build a matcher from a set of needles (typically `"<hex_hash>"` strings).
    fn new(needles: Vec<Vec<u8>>) -> Self {
        let mut prefix_map: HashMap<[u8; 4], Vec<Vec<u8>>> = HashMap::new();
        for needle in &needles {
            let prefix: [u8; 4] = if needle.len() >= 4 {
                needle[0..4].try_into().unwrap()
            } else {
                [0, 0, 0, 0]
            };
            prefix_map.entry(prefix).or_default().push(needle.clone());
        }
        Self { prefix_map }
    }

    /// Check whether any needle appears as a substring in `line`.
    ///
    /// Scans for `"` bytes (all needles start with `"`), then checks the 4-byte
    /// prefix against the lookup table. Only matching candidates get full comparison.
    /// False positives are harmless (they just cause an extra serde deserialize).
    #[inline]
    fn line_matches(&self, line: &[u8]) -> bool {
        let mut i = 0;
        let line_len = line.len();
        while i < line_len {
            if line[i] != b'"' {
                i += 1;
                continue;
            }
            // Found opening quote — look up 4-byte prefix
            if i + 4 <= line_len {
                let prefix: [u8; 4] = line[i..i + 4].try_into().unwrap_or([0, 0, 0, 0]);
                if let Some(candidates) = self.prefix_map.get(&prefix) {
                    for candidate in candidates {
                        let clen = candidate.len();
                        if i + clen <= line_len && &line[i..i + clen] == candidate.as_slice() {
                            return true;
                        }
                    }
                }
            }
            i += 1;
        }
        false
    }
}

/// Fast: find token-index entries whose `sources` overlap with `source_hashes`.
/// Uses mmap + substring pre-filter to avoid deserializing non-matching lines.
pub fn find_entries_by_sources_fast(
    ob_dir: &Path,
    tokenizer: &str,
    source_hashes: &HashSet<String>,
) -> Result<Vec<TokenIndexEntry>> {
    let files = get_merged_files(ob_dir, tokenizer)?;
    if files.is_empty() {
        return Ok(Vec::new());
    }

    let needles: Vec<Vec<u8>> = source_hashes
        .iter()
        .map(|h| {
            let mut v = Vec::with_capacity(h.len() + 2);
            v.push(b'"');
            v.extend_from_slice(h.as_bytes());
            v.push(b'"');
            v
        })
        .collect();
    let matcher = FastMatcher::new(needles);

    let mut results = Vec::new();
    for (_, path) in &files {
        let mmap = match crate::mmap_lines::mmap_file(path)? {
            Some(m) => m,
            None => continue,
        };
        for line in crate::mmap_lines::iter_lines(&mmap) {
            if !matcher.line_matches(line) {
                continue;
            }
            if let Ok(entry) = serde_json::from_slice::<TokenIndexEntry>(line) {
                if entry
                    .sources
                    .iter()
                    .any(|s| source_hashes.contains(s))
                {
                    results.push(entry);
                }
            }
        }
    }
    Ok(results)
}

/// Fast: revoke (toggle) entries whose sources overlap with `target_sources`.
/// Uses mmap + substring pre-filter; only deserializes matching lines.
/// Non-matching lines are copied to the output file as raw bytes (no serde).
pub fn revoke_by_sources_fast(
    ob_dir: &Path,
    tokenizer: &str,
    target_sources: &HashSet<String>,
) -> Result<usize> {
    let files = get_merged_files(ob_dir, tokenizer)?;
    if files.is_empty() {
        return Ok(0);
    }

    let needles: Vec<Vec<u8>> = target_sources
        .iter()
        .map(|h| {
            let mut v = Vec::with_capacity(h.len() + 2);
            v.push(b'"');
            v.extend_from_slice(h.as_bytes());
            v.push(b'"');
            v
        })
        .collect();
    let matcher = FastMatcher::new(needles);

    let mut total_toggled = 0usize;
    for (_, path) in &files {
        let mmap = match crate::mmap_lines::mmap_file(path)? {
            Some(m) => m,
            None => continue,
        };
        let data = &*mmap;

        let mut lines: Vec<(usize, usize)> = Vec::new();
        let mut pos: usize = 0;
        while pos < data.len() {
            while pos < data.len() && data[pos] == b'\n' {
                pos += 1;
            }
            if pos >= data.len() {
                break;
            }
            let start = pos;
            while pos < data.len() && data[pos] != b'\n' {
                pos += 1;
            }
            let end = pos;
            if end > start {
                lines.push((start, end));
            }
        }

        let mut toggled_in_file = 0usize;
        let mut output = Vec::<u8>::with_capacity(data.len());

        for (start, end) in &lines {
            let line = &data[*start..*end];
            if matcher.line_matches(line) {
                if let Ok(mut entry) = serde_json::from_slice::<TokenIndexEntry>(line) {
                    let matches = entry
                        .sources
                        .iter()
                        .any(|s| target_sources.contains(s));
                    if matches {
                        entry.revoked = !entry.revoked;
                        toggled_in_file += 1;
                        let serialized =
                            serde_json::to_string(&entry).expect("entry serialization");
                        output.extend_from_slice(serialized.as_bytes());
                        output.push(b'\n');
                        continue;
                    }
                }
                output.extend_from_slice(line);
                output.push(b'\n');
            } else {
                output.extend_from_slice(line);
                output.push(b'\n');
            }
        }

        if toggled_in_file > 0 {
            std::fs::write(path, &output)?;
            total_toggled += toggled_in_file;
        }
    }
    Ok(total_toggled)
}

/// Fast: generate the forget-set bitmask using mmap + substring matching.
/// Checks for `"revoked":true` substring (serde compact format) — no deserialization at all.
pub fn generate_forget_set_fast(ob_dir: &Path, tokenizer: &str) -> Result<Vec<u8>> {
    let files = get_merged_files(ob_dir, tokenizer)?;
    if files.is_empty() {
        return Ok(Vec::new());
    }

    let revoked_needle = b"\"revoked\":true";
    let mut total_entries = 0usize;
    let mut revoked_bits: Vec<u8> = Vec::new();

    for (_, path) in &files {
        let mmap = match crate::mmap_lines::mmap_file(path)? {
            Some(m) => m,
            None => continue,
        };
        for line in crate::mmap_lines::iter_lines(&mmap) {
            let bit_idx = total_entries;
            let byte_idx = bit_idx / 8;
            let bit_offset = (bit_idx % 8) as u8;
            if byte_idx >= revoked_bits.len() {
                revoked_bits.resize(byte_idx + 1, 0u8);
            }
            if line.windows(revoked_needle.len())
                .any(|w| w == revoked_needle)
            {
                revoked_bits[byte_idx] |= 1 << bit_offset;
            }
            total_entries += 1;
        }
    }

    Ok(revoked_bits)
}

pub fn token_status(ob_dir: &Path, tokenizer: &str) -> Result<TokenStatus> {
    let entries = read_merged(ob_dir, tokenizer)?;
    let total_entries = entries.len();
    let mut active_entries = 0usize;
    let mut revoked_entries = 0usize;
    let mut total_tokens = 0u64;

    for entry in &entries {
        total_tokens += entry.token_count;
        if entry.revoked {
            revoked_entries += 1;
        } else {
            active_entries += 1;
        }
    }

    Ok(TokenStatus {
        total_entries,
        active_entries,
        revoked_entries,
        total_tokens,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_ob_dir(tmp: &TempDir) -> PathBuf {
        let ob = tmp.path().join(".ob");
        std::fs::create_dir_all(&ob).unwrap();
        tmp.path().to_path_buf()
    }

    fn write_merged_file(
        ob_dir: &Path,
        tokenizer: &str,
        file_num: u64,
        entries: &[TokenIndexEntry],
    ) {
        let dir = ob_dir.join(".ob").join(format!("token-index.{}", tokenizer));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{:03}", file_num));
        let mut file = std::fs::File::create(&path).unwrap();
        for e in entries {
            writeln!(file, "{}", serde_json::to_string(e).unwrap()).unwrap();
        }
    }

    #[test]
    fn test_token_index_entry_serde() {
        let entry = TokenIndexEntry {
            token_count: 5,
            sources: vec!["a3f7b2c8d1".to_string()],
            tokenizer: "gpt2".to_string(),
            revoked: false,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: TokenIndexEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.token_count, 5);
        assert_eq!(parsed.sources, vec!["a3f7b2c8d1"]);
        assert_eq!(parsed.tokenizer, "gpt2");
        assert!(!parsed.revoked);
    }

    #[test]
    fn test_write_pid_creates_file() {
        let tmp = TempDir::new().unwrap();
        let ob_dir = make_ob_dir(&tmp);

        write_pid(&ob_dir, "gpt2", 42, &["src1".to_string(), "src2".to_string()]).unwrap();

        let pid_files = list_pid_files(&ob_dir, "gpt2").unwrap();
        assert_eq!(pid_files.len(), 1);

        let records = crate::storage::jsonl_read(&pid_files[0]).unwrap();
        assert_eq!(records.len(), 1);
        let entry: TokenIndexEntry =
            serde_json::from_value(records[0].clone()).unwrap();
        assert_eq!(entry.token_count, 42);
        assert_eq!(entry.sources, vec!["src1", "src2"]);
        assert_eq!(entry.tokenizer, "gpt2");
        assert!(!entry.revoked);
    }

    #[test]
    fn test_list_pid_files() {
        let tmp = TempDir::new().unwrap();
        let ob_dir = make_ob_dir(&tmp);

        write_pid(&ob_dir, "gpt2", 10, &["a".to_string()]).unwrap();
        write_pid(&ob_dir, "gpt2", 20, &["b".to_string()]).unwrap();
        write_pid(&ob_dir, "llama", 30, &["c".to_string()]).unwrap();

        let gpt2_files = list_pid_files(&ob_dir, "gpt2").unwrap();
        assert_eq!(gpt2_files.len(), 1);

        let llama_files = list_pid_files(&ob_dir, "llama").unwrap();
        assert_eq!(llama_files.len(), 1);

        let none = list_pid_files(&ob_dir, "bert").unwrap();
        assert!(none.is_empty());
    }

    #[test]
    fn test_read_merged() {
        let tmp = TempDir::new().unwrap();
        let ob_dir = make_ob_dir(&tmp);

        let entries1 = vec![
            TokenIndexEntry {
                token_count: 10,
                sources: vec!["a".to_string()],
                tokenizer: "gpt2".to_string(),
                revoked: false,
            },
        ];
        let entries2 = vec![
            TokenIndexEntry {
                token_count: 20,
                sources: vec!["b".to_string()],
                tokenizer: "gpt2".to_string(),
                revoked: false,
            },
        ];

        write_merged_file(&ob_dir, "gpt2", 0, &entries1);
        write_merged_file(&ob_dir, "gpt2", 1, &entries2);

        let result = read_merged(&ob_dir, "gpt2").unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].token_count, 10);
        assert_eq!(result[1].token_count, 20);
    }

    #[test]
    fn test_revoke_toggles() {
        let tmp = TempDir::new().unwrap();
        let ob_dir = make_ob_dir(&tmp);

        let entries = vec![
            TokenIndexEntry {
                token_count: 10,
                sources: vec!["src_a".to_string(), "src_b".to_string()],
                tokenizer: "gpt2".to_string(),
                revoked: false,
            },
            TokenIndexEntry {
                token_count: 20,
                sources: vec!["src_c".to_string()],
                tokenizer: "gpt2".to_string(),
                revoked: false,
            },
        ];

        write_merged_file(&ob_dir, "gpt2", 0, &entries);

        let mut targets = HashSet::new();
        targets.insert("src_a".to_string());

        let count = revoke_by_sources(&ob_dir, "gpt2", &targets).unwrap();
        assert_eq!(count, 1);

        let updated = read_merged(&ob_dir, "gpt2").unwrap();
        assert_eq!(updated.len(), 2);
        assert!(updated[0].revoked);
        assert!(!updated[1].revoked);
    }

    #[test]
    fn test_merge_pid_files() {
        let tmp = TempDir::new().unwrap();
        let ob_dir = make_ob_dir(&tmp);

        write_pid(&ob_dir, "gpt2", 10, &["a".to_string()]).unwrap();
        write_pid(&ob_dir, "gpt2", 20, &["b".to_string()]).unwrap();

        let (merged, deleted) = merge_pid_files(&ob_dir, "gpt2").unwrap();
        assert_eq!(merged, 2);
        assert_eq!(deleted, 1);

        let pids = list_pid_files(&ob_dir, "gpt2").unwrap();
        assert!(pids.is_empty());

        let entries = read_merged(&ob_dir, "gpt2").unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn test_list_tokenizers() {
        let tmp = TempDir::new().unwrap();
        let ob_dir = make_ob_dir(&tmp);

        write_pid(&ob_dir, "gpt2", 5, &["x".to_string()]).unwrap();

        let llama_dir = ob_dir.join(".ob").join("token-index.llama");
        std::fs::create_dir_all(&llama_dir).unwrap();

        let tokenizers = list_tokenizers(&ob_dir).unwrap();
        assert!(tokenizers.contains(&"gpt2".to_string()));
        assert!(tokenizers.contains(&"llama".to_string()));
    }

    #[test]
    fn test_archive_revoked() {
        let tmp = TempDir::new().unwrap();
        let ob_dir = make_ob_dir(&tmp);

        let entries = vec![
            TokenIndexEntry {
                token_count: 10,
                sources: vec!["a".to_string()],
                tokenizer: "gpt2".to_string(),
                revoked: true,
            },
            TokenIndexEntry {
                token_count: 20,
                sources: vec!["b".to_string()],
                tokenizer: "gpt2".to_string(),
                revoked: false,
            },
            TokenIndexEntry {
                token_count: 30,
                sources: vec!["c".to_string()],
                tokenizer: "gpt2".to_string(),
                revoked: true,
            },
        ];

        write_merged_file(&ob_dir, "gpt2", 0, &entries);

        let count = archive_revoked(&ob_dir, "gpt2").unwrap();
        assert_eq!(count, 2);

        let active = read_merged(&ob_dir, "gpt2").unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].token_count, 20);

        let archive_path = ob_dir
            .join(".ob")
            .join("archive")
            .join("token-index.gpt2.00");
        assert!(archive_path.exists());
        let archived = crate::storage::jsonl_read(&archive_path).unwrap();
        assert_eq!(archived.len(), 2);
    }

    #[test]
    fn test_token_status() {
        let tmp = TempDir::new().unwrap();
        let ob_dir = make_ob_dir(&tmp);

        let entries = vec![
            TokenIndexEntry {
                token_count: 10,
                sources: vec!["a".to_string()],
                tokenizer: "gpt2".to_string(),
                revoked: false,
            },
            TokenIndexEntry {
                token_count: 20,
                sources: vec!["b".to_string()],
                tokenizer: "gpt2".to_string(),
                revoked: true,
            },
            TokenIndexEntry {
                token_count: 30,
                sources: vec!["c".to_string()],
                tokenizer: "gpt2".to_string(),
                revoked: false,
            },
            TokenIndexEntry {
                token_count: 40,
                sources: vec!["d".to_string()],
                tokenizer: "gpt2".to_string(),
                revoked: true,
            },
        ];

        write_merged_file(&ob_dir, "gpt2", 0, &entries);

        let status = token_status(&ob_dir, "gpt2").unwrap();
        assert_eq!(status.total_entries, 4);
        assert_eq!(status.active_entries, 2);
        assert_eq!(status.revoked_entries, 2);
        assert_eq!(status.total_tokens, 100);
    }
}
