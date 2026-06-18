use anyhow::{bail, ensure, Context, Result};
use std::collections::HashSet;
use std::io::Write;
use std::path::Path;

const MAGIC: &[u8; 9] = b"OBIDXTI01";
const MAGIC_LEN: usize = 9;
const HEADER_SIZE: usize = 64;
const ENTRY_SIZE: usize = 16;
const BUCKET_COUNT: usize = 256;

/// Reference to a JSONL line within the merged token-index files.
#[derive(Debug, Clone)]
pub struct EntryRef {
    pub jsonl_file_number: u16,
    pub jsonl_byte_offset: u32,
    pub jsonl_length: u32,
    pub revoked: bool,
}

/// Read-only handle to a token binary index file (OBIDXTI01).
pub struct TokenBinIndex {
    path: std::path::PathBuf,
    data: memmap2::Mmap,
    entry_count: u32,
    entry_table_offset: usize,
    #[allow(dead_code)]
    source_index_offset: usize,
    bucket_table_offset: usize,
}

/// Statistics returned after building the binary index.
#[derive(Debug)]
pub struct BuildStats {
    pub entry_count: u32,
    pub unique_source_count: u32,
}

/// Builder for the token binary index.
pub struct TokenBinIndexBuilder;

/// Source index entry used during building.
struct SourceEntry {
    source_hash: [u8; 32],
    entry_indices: Vec<u32>,
}

impl TokenBinIndex {
    /// Open an existing binary index, returns `Ok(None)` if the file does not exist.
    pub fn open(ob_dir: &Path, tokenizer: &str) -> Result<Option<Self>> {
        let path = ob_dir
            .join(".ob")
            .join(format!("token-index.{}.bin", tokenizer));
        if !path.exists() {
            return Ok(None);
        }

        let file = std::fs::File::open(&path)
            .with_context(|| format!("opening {}", path.display()))?;
        let metadata = file.metadata()?;
        if metadata.len() < HEADER_SIZE as u64 {
            bail!("token binary index file too small: {} bytes", metadata.len());
        }

        let mmap = unsafe { memmap2::Mmap::map(&file)? };

        if &mmap[0..MAGIC_LEN] != MAGIC {
            bail!("token binary index magic mismatch");
        }

        let version = u32::from_le_bytes(mmap[MAGIC_LEN..MAGIC_LEN + 4].try_into().unwrap());
        ensure!(version == 1, "unsupported token binary index version: {}", version);

        let entry_count = u32::from_le_bytes(mmap[MAGIC_LEN + 4..MAGIC_LEN + 8].try_into().unwrap());
        let _unique_source_count = u32::from_le_bytes(mmap[MAGIC_LEN + 8..MAGIC_LEN + 12].try_into().unwrap());
        let entry_table_offset = u32::from_le_bytes(mmap[MAGIC_LEN + 12..MAGIC_LEN + 16].try_into().unwrap()) as usize;
        let source_index_offset = u32::from_le_bytes(mmap[MAGIC_LEN + 16..MAGIC_LEN + 20].try_into().unwrap()) as usize;
        let bucket_table_offset = u32::from_le_bytes(mmap[MAGIC_LEN + 20..MAGIC_LEN + 24].try_into().unwrap()) as usize;

        Ok(Some(Self {
            path,
            data: mmap,
            entry_count,
            entry_table_offset,
            source_index_offset,
            bucket_table_offset,
        }))
    }

    /// Check whether the binary index is fresh (newer mtime than all JSONL merged files).
    pub fn is_fresh(&self) -> bool {
        let bin_mtime = match std::fs::metadata(&self.path).and_then(|m| m.modified()) {
            Ok(t) => t,
            Err(_) => return false,
        };

        // Extract ob_dir from path: path = ob_dir/.ob/token-index.{tokenizer}.bin
        let ob_dot = match self.path.parent() {
            Some(p) => p,
            None => return false,
        };

        // Determine tokenizer name from filename
        let filename = match self.path.file_name() {
            Some(n) => n.to_string_lossy().to_string(),
            None => return false,
        };
        let tokenizer = match filename
            .strip_prefix("token-index.")
            .and_then(|s| s.strip_suffix(".bin"))
        {
            Some(t) => t,
            None => return false,
        };

        let merged_dir = ob_dot.join(format!("token-index.{}", tokenizer));
        if let Ok(entries) = std::fs::read_dir(&merged_dir) {
            for entry in entries.flatten() {
                if entry.path().is_file() {
                    if let Ok(m) = entry.metadata().and_then(|m| m.modified()) {
                        if m > bin_mtime {
                            return false;
                        }
                    }
                }
                }
        }

        true
    }

    /// Look up entry indices matching a single source hash.
    pub fn lookup_by_source(&self, source_hash: &[u8; 32]) -> Vec<u32> {
        let data = &self.data;
        let bucket = source_hash[0] as usize;

        let bucket_off = self.bucket_table_offset + bucket * 8;
        let start = u64::from_le_bytes(data[bucket_off..bucket_off + 8].try_into().unwrap()) as usize;
        if start == 0 {
            return Vec::new();
        }

        // Find end of this bucket
        let end = self.find_bucket_end(bucket);

        // Binary search within the bucket for exact source_hash match
        let mut lo = start;
        let mut hi = end;
        while lo < hi {
            if lo + 32 + 2 > hi {
                break;
            }
            let mid_hash = &data[lo..lo + 32];
            let match_count =
                u16::from_le_bytes(data[lo + 32..lo + 34].try_into().unwrap()) as usize;
            let entry_size = 32 + 2 + 4 * match_count;

            match mid_hash.cmp(source_hash) {
                std::cmp::Ordering::Equal => {
                    let mut result = Vec::with_capacity(match_count);
                    for i in 0..match_count {
                        let off = lo + 34 + i * 4;
                        result.push(u32::from_le_bytes(
                            data[off..off + 4].try_into().unwrap(),
                        ));
                    }
                    return result;
                }
                std::cmp::Ordering::Less => lo += entry_size,
                std::cmp::Ordering::Greater => hi = lo,
            }
        }

        Vec::new()
    }

    /// Look up entry indices matching any of the given source hashes.
    pub fn lookup_by_sources(&self, source_hashes: &HashSet<[u8; 32]>) -> HashSet<u32> {
        let mut result = HashSet::new();
        for hash in source_hashes {
            for idx in self.lookup_by_source(hash) {
                result.insert(idx);
            }
        }
        result
    }

    /// Get a reference to a specific entry by index.
    pub fn get_entry_ref(&self, entry_idx: u32) -> Option<EntryRef> {
        if entry_idx >= self.entry_count {
            return None;
        }
        let off = self.entry_table_offset + entry_idx as usize * ENTRY_SIZE;
        if off + ENTRY_SIZE > self.data.len() {
            return None;
        }

        let jsonl_file_number =
            u16::from_le_bytes(self.data[off..off + 2].try_into().unwrap());
        let jsonl_byte_offset =
            u32::from_le_bytes(self.data[off + 2..off + 6].try_into().unwrap());
        let jsonl_length =
            u32::from_le_bytes(self.data[off + 6..off + 10].try_into().unwrap());
        let revoked = self.data[off + 10] != 0;

        Some(EntryRef {
            jsonl_file_number,
            jsonl_byte_offset,
            jsonl_length,
            revoked,
        })
    }

    /// Set the revoked flag for a specific entry in-place (via MmapMut).
    pub fn set_revoked_batch(&self, entries: &[(u32, bool)]) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.path)
            .with_context(|| format!("opening {} for write", self.path.display()))?;
        let mut mmap_mut = unsafe { memmap2::MmapMut::map_mut(&file)? };
        for &(entry_idx, revoked) in entries {
            if entry_idx >= self.entry_count {
                continue;
            }
            let off = self.entry_table_offset + entry_idx as usize * ENTRY_SIZE + 10;
            mmap_mut[off] = if revoked { 1 } else { 0 };
        }
        mmap_mut.flush()?;
        Ok(())
    }

    /// Generate a forget-set bitmask by scanning revoked flags.
    /// No JSONL deserialization needed.
    pub fn generate_forget_set(&self) -> Vec<u8> {
        let byte_count = (self.entry_count as usize + 7) / 8;
        let mut bitmask = vec![0u8; byte_count];
        for i in 0..self.entry_count {
            let off = self.entry_table_offset + i as usize * ENTRY_SIZE + 10;
            if self.data[off] != 0 {
                bitmask[i as usize / 8] |= 1 << (i as usize % 8);
            }
        }
        bitmask
    }

    /// Return the total number of entries in the index.
    pub fn entry_count(&self) -> u32 {
        self.entry_count
    }

    /// Find the end byte offset of a bucket (first byte of next non-empty bucket,
    /// or bucket_table_offset if none).
    fn find_bucket_end(&self, bucket: usize) -> usize {
        let data = &self.data;
        for i in (bucket + 1)..BUCKET_COUNT {
            let off = self.bucket_table_offset + i * 8;
            let start = u64::from_le_bytes(data[off..off + 8].try_into().unwrap()) as usize;
            if start != 0 {
                return start;
            }
        }
        self.bucket_table_offset
    }
}

impl TokenBinIndexBuilder {
    /// Build a token binary index from the merged JSONL files.
    ///
    /// * `ob_dir` – repository root (containing `.ob/`)
    /// * `tokenizer` – tokenizer name (e.g. `"gpt2"`)
    /// * `entries` – deserialized token index entries in order
    /// * `jsonl_offsets` – parallel array of (file_number, byte_offset, byte_length) for each entry
    pub fn build(
        ob_dir: &Path,
        tokenizer: &str,
        entries: &[crate::token_index::TokenIndexEntry],
        jsonl_offsets: &[(u16, u32, u32)],
    ) -> Result<BuildStats> {
        ensure!(
            entries.len() == jsonl_offsets.len(),
            "entries and jsonl_offsets must have the same length ({} vs {})",
            entries.len(),
            jsonl_offsets.len()
        );

        let entry_count = entries.len() as u32;

        // Build inverted index: source_hash -> entry indices
        let mut source_map: std::collections::HashMap<[u8; 32], Vec<u32>> =
            std::collections::HashMap::new();

        for (i, entry) in entries.iter().enumerate() {
            for source_hex in &entry.sources {
                let source_bytes: [u8; 32] = match hex::decode(source_hex) {
                    Ok(b) => match b.try_into() {
                        Ok(a) => a,
                        Err(_) => continue,
                    },
                    Err(_) => continue,
                };
                source_map
                    .entry(source_bytes)
                    .or_default()
                    .push(i as u32);
            }
        }

        let unique_source_count = source_map.len() as u32;

        // Sort source entries by hash
        let mut source_entries: Vec<SourceEntry> = source_map
            .into_iter()
            .map(|(source_hash, entry_indices)| SourceEntry {
                source_hash,
                entry_indices,
            })
            .collect();
        source_entries.sort_by(|a, b| a.source_hash.cmp(&b.source_hash));

        // Compute section offsets
        let entry_table_offset = HEADER_SIZE as u32;
        let entry_table_size = entry_count * ENTRY_SIZE as u32;
        let source_index_offset = entry_table_offset + entry_table_size;

        // Compute source index size
        let mut source_index_size: u32 = 0;
        for se in &source_entries {
            source_index_size += 32 + 2 + 4 * se.entry_indices.len() as u32;
        }

        let bucket_table_offset = source_index_offset + source_index_size;

        // Write file
        let out_path = ob_dir
            .join(".ob")
            .join(format!("token-index.{}.bin", tokenizer));

        let mut file = std::fs::File::create(&out_path)
            .with_context(|| format!("creating {}", out_path.display()))?;

        // Header (64 bytes)
        file.write_all(MAGIC)?;
        file.write_all(&1u32.to_le_bytes())?;
        file.write_all(&entry_count.to_le_bytes())?;
        file.write_all(&unique_source_count.to_le_bytes())?;
        file.write_all(&entry_table_offset.to_le_bytes())?;
        file.write_all(&source_index_offset.to_le_bytes())?;
        file.write_all(&bucket_table_offset.to_le_bytes())?;
        let reserved_len = HEADER_SIZE - MAGIC_LEN - 6 * 4;
        file.write_all(&vec![0u8; reserved_len])?;

        // Entry table
        for (i, entry) in entries.iter().enumerate() {
            let (file_num, byte_off, byte_len) = jsonl_offsets[i];
            file.write_all(&file_num.to_le_bytes())?;
            file.write_all(&byte_off.to_le_bytes())?;
            file.write_all(&byte_len.to_le_bytes())?;
            let revoked_byte: u8 = if entry.revoked { 1 } else { 0 };
            file.write_all(&[revoked_byte])?;
            file.write_all(&[0u8; 5])?; // padding
        }

        // Source index
        for se in &source_entries {
            file.write_all(&se.source_hash)?;
            let count = se.entry_indices.len() as u16;
            file.write_all(&count.to_le_bytes())?;
            for &idx in &se.entry_indices {
                file.write_all(&idx.to_le_bytes())?;
            }
        }

        // Bucket table (256 × u64)
        let mut bucket_offsets: [u64; BUCKET_COUNT] = [0; BUCKET_COUNT];
        let mut current_offset = source_index_offset;
        let mut last_bucket: Option<usize> = None;
        for se in &source_entries {
            let bucket = se.source_hash[0] as usize;
            if last_bucket != Some(bucket) {
                bucket_offsets[bucket] = current_offset as u64;
                last_bucket = Some(bucket);
            }
            current_offset += 32 + 2 + 4 * se.entry_indices.len() as u32;
        }

        for off in &bucket_offsets {
            file.write_all(&off.to_le_bytes())?;
        }

        // Verify by re-opening
        let verify = TokenBinIndex::open(ob_dir, tokenizer)?
            .context("failed to re-open token binary index for verification")?;
        ensure!(
            verify.entry_count == entry_count,
            "verification failed: expected {} entries, stored {}",
            entry_count,
            verify.entry_count
        );

        Ok(BuildStats {
            entry_count,
            unique_source_count,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token_index::TokenIndexEntry;
    use tempfile::TempDir;

    fn make_ob_dir(tmp: &TempDir) -> std::path::PathBuf {
        let ob = tmp.path().join(".ob");
        std::fs::create_dir_all(&ob).unwrap();
        tmp.path().to_path_buf()
    }

    fn simple_entry(token_count: u64, sources: &[&str], revoked: bool) -> TokenIndexEntry {
        TokenIndexEntry {
            token_count,
            sources: sources.iter().map(|s| s.to_string()).collect(),
            tokenizer: "gpt2".to_string(),
            revoked,
        }
    }

    fn write_merged_jsonl(
        ob_dir: &Path,
        tokenizer: &str,
        file_num: u16,
        entries: &[TokenIndexEntry],
    ) -> Vec<(u16, u32, u32)> {
        let dir = ob_dir.join(".ob").join(format!("token-index.{}", tokenizer));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{:03}", file_num));

        let mut offsets = Vec::new();
        let mut file = std::fs::File::create(&path).unwrap();
        for e in entries {
            let line = serde_json::to_string(e).unwrap();
            let byte_offset = file.metadata().unwrap().len() as u32;
            use std::io::Write;
            write!(file, "{}\n", line).unwrap();
            let byte_len = (file.metadata().unwrap().len() as u32) - byte_offset;
            offsets.push((file_num, byte_offset, byte_len));
        }
        offsets
    }

    #[test]
    fn test_build_and_open() {
        let tmp = TempDir::new().unwrap();
        let ob_dir = make_ob_dir(&tmp);

        let hash_a = "aa".repeat(32);
        let hash_b = "bb".repeat(32);

        let entries = vec![
            simple_entry(10, &[&hash_a], false),
            simple_entry(20, &[&hash_b, &hash_a], false),
            simple_entry(30, &[&hash_b], true),
        ];

        let offsets = write_merged_jsonl(&ob_dir, "gpt2", 0, &entries);

        let stats = TokenBinIndexBuilder::build(&ob_dir, "gpt2", &entries, &offsets).unwrap();
        assert_eq!(stats.entry_count, 3);
        assert_eq!(stats.unique_source_count, 2);

        let idx = TokenBinIndex::open(&ob_dir, "gpt2").unwrap().unwrap();
        assert_eq!(idx.entry_count(), 3);
    }

    #[test]
    fn test_lookup_by_source() {
        let tmp = TempDir::new().unwrap();
        let ob_dir = make_ob_dir(&tmp);

        let hash_a = "aa".repeat(32);
        let hash_b = "bb".repeat(32);
        let hash_c = "cc".repeat(32); // not used

        let entries = vec![
            simple_entry(10, &[&hash_a], false),
            simple_entry(20, &[&hash_b, &hash_a], false),
            simple_entry(30, &[&hash_b], true),
        ];

        let offsets = write_merged_jsonl(&ob_dir, "gpt2", 0, &entries);
        TokenBinIndexBuilder::build(&ob_dir, "gpt2", &entries, &offsets).unwrap();

        let idx = TokenBinIndex::open(&ob_dir, "gpt2").unwrap().unwrap();

        let hash_a_bytes = hex::decode(&hash_a).unwrap().try_into().unwrap();
        let hash_b_bytes = hex::decode(&hash_b).unwrap().try_into().unwrap();
        let hash_c_bytes = hex::decode(&hash_c).unwrap().try_into().unwrap();

        let a_entries = idx.lookup_by_source(&hash_a_bytes);
        assert_eq!(a_entries.len(), 2);
        assert!(a_entries.contains(&0));
        assert!(a_entries.contains(&1));

        let b_entries = idx.lookup_by_source(&hash_b_bytes);
        assert_eq!(b_entries.len(), 2);
        assert!(b_entries.contains(&1));
        assert!(b_entries.contains(&2));

        let c_entries = idx.lookup_by_source(&hash_c_bytes);
        assert!(c_entries.is_empty());
    }

    #[test]
    fn test_lookup_by_sources() {
        let tmp = TempDir::new().unwrap();
        let ob_dir = make_ob_dir(&tmp);

        let hash_a = "aa".repeat(32);
        let hash_b = "bb".repeat(32);

        let entries = vec![
            simple_entry(10, &[&hash_a], false),
            simple_entry(20, &[&hash_b], false),
        ];

        let offsets = write_merged_jsonl(&ob_dir, "gpt2", 0, &entries);
        TokenBinIndexBuilder::build(&ob_dir, "gpt2", &entries, &offsets).unwrap();

        let idx = TokenBinIndex::open(&ob_dir, "gpt2").unwrap().unwrap();

        let hash_a_bytes: [u8; 32] = hex::decode(&hash_a).unwrap().try_into().unwrap();
        let hash_b_bytes: [u8; 32] = hex::decode(&hash_b).unwrap().try_into().unwrap();

        let mut sources = HashSet::new();
        sources.insert(hash_a_bytes);

        let result = idx.lookup_by_sources(&sources);
        assert_eq!(result.len(), 1);
        assert!(result.contains(&0));

        sources.insert(hash_b_bytes);
        let result = idx.lookup_by_sources(&sources);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_get_entry_ref() {
        let tmp = TempDir::new().unwrap();
        let ob_dir = make_ob_dir(&tmp);

        let hash_a = "aa".repeat(32);

        let entries = vec![
            simple_entry(10, &[&hash_a], false),
            simple_entry(20, &[&hash_a], true),
        ];

        let offsets = write_merged_jsonl(&ob_dir, "gpt2", 0, &entries);
        TokenBinIndexBuilder::build(&ob_dir, "gpt2", &entries, &offsets).unwrap();

        let idx = TokenBinIndex::open(&ob_dir, "gpt2").unwrap().unwrap();

        let e0 = idx.get_entry_ref(0).unwrap();
        assert_eq!(e0.jsonl_file_number, 0);
        assert!(!e0.revoked);

        let e1 = idx.get_entry_ref(1).unwrap();
        assert!(e1.revoked);

        assert!(idx.get_entry_ref(999).is_none());
    }

    #[test]
    fn test_set_revoked() {
        let tmp = TempDir::new().unwrap();
        let ob_dir = make_ob_dir(&tmp);

        let hash_a = "aa".repeat(32);

        let entries = vec![
            simple_entry(10, &[&hash_a], false),
            simple_entry(20, &[&hash_a], false),
        ];

        let offsets = write_merged_jsonl(&ob_dir, "gpt2", 0, &entries);
        TokenBinIndexBuilder::build(&ob_dir, "gpt2", &entries, &offsets).unwrap();

        let idx = TokenBinIndex::open(&ob_dir, "gpt2").unwrap().unwrap();

        assert!(!idx.get_entry_ref(0).unwrap().revoked);

        idx.set_revoked_batch(&[(0, true)]).unwrap();

        // Re-open to verify persistence
        let idx2 = TokenBinIndex::open(&ob_dir, "gpt2").unwrap().unwrap();
        assert!(idx2.get_entry_ref(0).unwrap().revoked);
        assert!(!idx2.get_entry_ref(1).unwrap().revoked);

        // Toggle back
        idx2.set_revoked_batch(&[(0, false)]).unwrap();

        let idx3 = TokenBinIndex::open(&ob_dir, "gpt2").unwrap().unwrap();
        assert!(!idx3.get_entry_ref(0).unwrap().revoked);
    }

    #[test]
    fn test_generate_forget_set() {
        let tmp = TempDir::new().unwrap();
        let ob_dir = make_ob_dir(&tmp);

        let hash_a = "aa".repeat(32);

        let entries = vec![
            simple_entry(10, &[&hash_a], false),
            simple_entry(20, &[&hash_a], true),
            simple_entry(30, &[&hash_a], false),
            simple_entry(40, &[&hash_a], true),
            simple_entry(50, &[&hash_a], true),
            simple_entry(60, &[&hash_a], false),
            simple_entry(70, &[&hash_a], false),
            simple_entry(80, &[&hash_a], true),
            simple_entry(90, &[&hash_a], false),
        ];

        let offsets = write_merged_jsonl(&ob_dir, "gpt2", 0, &entries);
        TokenBinIndexBuilder::build(&ob_dir, "gpt2", &entries, &offsets).unwrap();

        let idx = TokenBinIndex::open(&ob_dir, "gpt2").unwrap().unwrap();
        let bitmask = idx.generate_forget_set();

        assert_eq!(bitmask.len(), 2);
        // Revoked at indices 1,3,4,7 → byte 0 = 0b10011010
        assert_eq!(bitmask[0], 0b10011010);
        // Index 8 active → byte 1 = 0
        assert_eq!(bitmask[1], 0b00000000);
    }

    #[test]
    fn test_is_fresh() {
        let tmp = TempDir::new().unwrap();
        let ob_dir = make_ob_dir(&tmp);

        let hash_a = "aa".repeat(32);
        let entries = vec![simple_entry(10, &[&hash_a], false)];
        let offsets = write_merged_jsonl(&ob_dir, "gpt2", 0, &entries);
        TokenBinIndexBuilder::build(&ob_dir, "gpt2", &entries, &offsets).unwrap();

        let idx = TokenBinIndex::open(&ob_dir, "gpt2").unwrap().unwrap();
        assert!(idx.is_fresh());

        // Touch a JSONL file to make it newer
        std::thread::sleep(std::time::Duration::from_millis(10));
        let jsonl_path = ob_dir.join(".ob").join("token-index.gpt2").join("000");
        if jsonl_path.exists() {
            let now = std::time::SystemTime::now();
            let _ = std::fs::File::create(&jsonl_path)
                .and_then(|mut f| {
                    use std::io::Write;
                    writeln!(f, "{}", serde_json::to_string(&entries[0]).unwrap())
                });
            // Set mtime to future
            let _ = std::fs::File::open(&jsonl_path).and_then(|f| {
                f.set_modified(now + std::time::Duration::from_secs(10))
            });
        }

        assert!(!idx.is_fresh());
    }

    #[test]
    fn test_open_nonexistent() {
        let tmp = TempDir::new().unwrap();
        let ob_dir = make_ob_dir(&tmp);
        let result = TokenBinIndex::open(&ob_dir, "gpt2").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_multi_file_build() {
        let tmp = TempDir::new().unwrap();
        let ob_dir = make_ob_dir(&tmp);

        let hash_a = "aa".repeat(32);
        let hash_b = "bb".repeat(32);

        let entries0 = vec![simple_entry(10, &[&hash_a], false)];
        let entries1 = vec![
            simple_entry(20, &[&hash_b], false),
            simple_entry(30, &[&hash_a, &hash_b], true),
        ];

        let offsets0 = write_merged_jsonl(&ob_dir, "gpt2", 0, &entries0);
        let offsets1 = write_merged_jsonl(&ob_dir, "gpt2", 1, &entries1);

        let all_entries: Vec<TokenIndexEntry> = entries0
            .into_iter()
            .chain(entries1.into_iter())
            .collect();
        let all_offsets: Vec<(u16, u32, u32)> = offsets0
            .into_iter()
            .chain(offsets1.into_iter())
            .collect();

        let stats =
            TokenBinIndexBuilder::build(&ob_dir, "gpt2", &all_entries, &all_offsets).unwrap();
        assert_eq!(stats.entry_count, 3);
        assert_eq!(stats.unique_source_count, 2);

        let idx = TokenBinIndex::open(&ob_dir, "gpt2").unwrap().unwrap();

        let hash_a_bytes: [u8; 32] = hex::decode(&hash_a).unwrap().try_into().unwrap();
        let a = idx.lookup_by_source(&hash_a_bytes);
        assert_eq!(a.len(), 2);

        // Check entry refs have correct file numbers
        let e0 = idx.get_entry_ref(0).unwrap();
        assert_eq!(e0.jsonl_file_number, 0);
        assert!(!e0.revoked);

        let e2 = idx.get_entry_ref(2).unwrap();
        assert_eq!(e2.jsonl_file_number, 1);
        assert!(e2.revoked);
    }
}
