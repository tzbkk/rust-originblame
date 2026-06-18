use anyhow::{bail, ensure, Context, Result};
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

const MAGIC: &[u8; 8] = b"OBIDXF02";
const MAGIC_V01: &[u8; 8] = b"OBIDXF01";
const HEADER_SIZE: usize = 24;
const ID_SIZE: usize = 32;
const TABLE_SIZE: usize = 256 * 8;

#[derive(Debug, Clone, PartialEq)]
pub enum IndexRef {
    DocumentShard { prefix: u8 },
    TokenIndexRange {
        tokenizer: String,
        file_number: u16,
        byte_offset: u32,
        length: u32,
    },
}

impl IndexRef {
    fn serialized_size(&self) -> usize {
        match self {
            IndexRef::DocumentShard { .. } => 2,
            IndexRef::TokenIndexRange { tokenizer, .. } => 1 + 1 + tokenizer.len() + 2 + 4 + 4,
        }
    }

    fn to_bytes(&self) -> Vec<u8> {
        match self {
            IndexRef::DocumentShard { prefix } => {
                vec![0x00, *prefix]
            }
            IndexRef::TokenIndexRange {
                tokenizer,
                file_number,
                byte_offset,
                length,
            } => {
                let name_bytes = tokenizer.as_bytes();
                let name_len = name_bytes.len() as u8;
                let mut buf = Vec::with_capacity(1 + 1 + name_bytes.len() + 2 + 4 + 4);
                buf.push(0x01);
                buf.push(name_len);
                buf.extend_from_slice(name_bytes);
                buf.extend_from_slice(&file_number.to_le_bytes());
                buf.extend_from_slice(&byte_offset.to_le_bytes());
                buf.extend_from_slice(&length.to_le_bytes());
                buf
            }
        }
    }

    fn from_bytes(data: &[u8], pos: usize) -> Option<(IndexRef, usize)> {
        if pos >= data.len() {
            return None;
        }
        match data[pos] {
            0x00 => {
                if pos + 1 >= data.len() {
                    return None;
                }
                Some((IndexRef::DocumentShard { prefix: data[pos + 1] }, 2))
            }
            0x01 => {
                if pos + 1 >= data.len() {
                    return None;
                }
                let name_len = data[pos + 1] as usize;
                let name_start = pos + 2;
                let name_end = name_start + name_len;
                if name_end + 2 + 4 + 4 > data.len() {
                    return None;
                }
                let tokenizer = String::from_utf8_lossy(&data[name_start..name_end]).to_string();
                let file_number =
                    u16::from_le_bytes(data[name_end..name_end + 2].try_into().unwrap());
                let byte_offset =
                    u32::from_le_bytes(data[name_end + 2..name_end + 6].try_into().unwrap());
                let length =
                    u32::from_le_bytes(data[name_end + 6..name_end + 10].try_into().unwrap());
                let consumed = 1 + 1 + name_len + 2 + 4 + 4;
                Some((
                    IndexRef::TokenIndexRange {
                        tokenizer,
                        file_number,
                        byte_offset,
                        length,
                    },
                    consumed,
                ))
            }
            _ => None,
        }
    }
}

pub struct BinaryIndex {
    data: memmap2::Mmap,
    line_count: u32,
    line_offsets_start: usize,
}

#[derive(Debug)]
pub struct BuildStats {
    pub entry_count: u32,
}

struct IndexEntry {
    id_hash: [u8; 32],
    refs: Vec<IndexRef>,
}

impl IndexEntry {
    fn refs_serialized_len(&self) -> usize {
        self.refs.iter().map(|r| r.serialized_size()).sum()
    }
}

impl BinaryIndex {
    pub fn open(ob_dir: &Path) -> Result<Option<Self>> {
        let path = ob_dir.join(".ob").join("index.bin");
        if !path.exists() {
            return Ok(None);
        }
        let file =
            std::fs::File::open(&path).with_context(|| format!("opening {}", path.display()))?;
        let metadata = file.metadata()?;
        if metadata.len() < (HEADER_SIZE + TABLE_SIZE) as u64 {
            bail!("binary index file too small: {} bytes", metadata.len());
        }
        let mmap = unsafe { memmap2::Mmap::map(&file)? };
        if &mmap[0..8] == MAGIC_V01 {
            bail!(
                "binary index format OBIDXF01 is no longer supported; run `ob index build` to rebuild"
            );
        }
        if &mmap[0..8] != MAGIC {
            bail!("binary index magic mismatch");
        }

        let sparse_table_end = {
            let offset_table_off = read_u64_le(&mmap, 16) as usize;
            offset_table_off + TABLE_SIZE
        };
        let file_len = mmap.len();
        let (line_count, line_offsets_start) = if file_len > sparse_table_end + 4 {
            let lc = u32::from_le_bytes(
                mmap[sparse_table_end..sparse_table_end + 4]
                    .try_into()
                    .unwrap(),
            );
            (lc, sparse_table_end + 4)
        } else {
            (0u32, 0usize)
        };

        Ok(Some(Self {
            data: mmap,
            line_count,
            line_offsets_start,
        }))
    }

    pub fn lookup(&self, id_hex: &str) -> Vec<IndexRef> {
        let id_bytes = match hex_to_bytes(id_hex) {
            Ok(b) => b,
            Err(_) => return Vec::new(),
        };

        let data = &self.data;
        let offset_table_off = read_u64_le(data, 16) as usize;
        let first_byte = id_bytes[0] as usize;

        let table_entry_off = offset_table_off + first_byte * 8;
        let start = read_u64_le(data, table_entry_off) as usize;
        if start == 0 {
            return Vec::new();
        }

        let end = self.find_end_offset(offset_table_off, first_byte);

        let mut lo = start;
        let mut hi = end;
        while lo < hi {
            if lo + ID_SIZE + 2 > hi {
                break;
            }
            let ref_data_size =
                u16::from_le_bytes([data[lo + ID_SIZE], data[lo + ID_SIZE + 1]]) as usize;
            let entry_len = ID_SIZE + 2 + ref_data_size;

            match data[lo..lo + ID_SIZE].cmp(&id_bytes) {
                std::cmp::Ordering::Equal => {
                    return parse_refs(&data[lo + ID_SIZE + 2..lo + ID_SIZE + 2 + ref_data_size]);
                }
                std::cmp::Ordering::Less => lo += entry_len,
                std::cmp::Ordering::Greater => hi = lo,
            }
        }

        Vec::new()
    }

    fn find_end_offset(&self, offset_table_off: usize, first_byte: usize) -> usize {
        let data = &self.data;
        for i in (first_byte + 1)..256 {
            let off = read_u64_le(data, offset_table_off + i * 8) as usize;
            if off != 0 {
                return off;
            }
        }
        offset_table_off
    }

    pub fn get_line_offset(&self, line_num: usize) -> Option<u64> {
        if line_num >= self.line_count as usize {
            return None;
        }
        let off = self.line_offsets_start + line_num * 4;
        Some(u32::from_le_bytes(self.data[off..off + 4].try_into().unwrap()) as u64)
    }

    pub fn append_line_offsets(ob_dir: &Path, data_file: &Path) -> Result<()> {
        let data_mmap = match crate::mmap_lines::mmap_file(data_file)? {
            Some(m) => m,
            None => return Ok(()),
        };

        let mut line_offsets: Vec<u32> = Vec::new();
        line_offsets.push(0u32);
        for i in 0..data_mmap.len() {
            if data_mmap[i] == b'\n' && i + 1 < data_mmap.len() {
                line_offsets.push((i + 1) as u32);
            }
        }

        let index_path = ob_dir.join(".ob").join("index.bin");
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&index_path)
            .with_context(|| format!("opening {} for append", index_path.display()))?;
        let line_count = line_offsets.len() as u32;
        file.write_all(&line_count.to_le_bytes())?;
        for off in &line_offsets {
            file.write_all(&off.to_le_bytes())?;
        }

        Ok(())
    }

    pub fn build(ob_dir: &Path, entries: Vec<([u8; 32], Vec<IndexRef>)>) -> Result<BuildStats> {
        let mut entries: Vec<IndexEntry> = entries
            .into_iter()
            .map(|(id_hash, refs)| IndexEntry { id_hash, refs })
            .collect();

        entries.sort_by(|a, b| a.id_hash.cmp(&b.id_hash));
        let entry_count = entries.len() as u32;

        let out_path = ob_dir.join(".ob").join("index.bin");
        let mut file = std::fs::File::create(&out_path)
            .with_context(|| format!("creating {}", out_path.display()))?;

        file.write_all(MAGIC)?;
        file.write_all(&entry_count.to_le_bytes())?;
        file.write_all(&[0u8; 4])?;
        file.write_all(&0u64.to_le_bytes())?;

        let mut prefix_offsets: [u64; 256] = [0; 256];
        let mut current_offset = HEADER_SIZE;
        let mut last_prefix: Option<u8> = None;

        for entry in &entries {
            let prefix = entry.id_hash[0];
            if last_prefix != Some(prefix) {
                prefix_offsets[prefix as usize] = current_offset as u64;
                last_prefix = Some(prefix);
            }
            current_offset += ID_SIZE + 2 + entry.refs_serialized_len();
        }

        let offset_table_off = current_offset as u64;

        for entry in &entries {
            file.write_all(&entry.id_hash)?;
            let refs_len = entry.refs_serialized_len() as u16;
            file.write_all(&refs_len.to_le_bytes())?;
            for r in &entry.refs {
                file.write_all(&r.to_bytes())?;
            }
        }

        for off in &prefix_offsets {
            file.write_all(&off.to_le_bytes())?;
        }

        {
            let mut file = std::fs::OpenOptions::new().write(true).open(&out_path)?;
            file.seek(SeekFrom::Start(16))?;
            file.write_all(&offset_table_off.to_le_bytes())?;
        }

        let verify =
            Self::open(ob_dir)?.context("failed to re-open binary index for verification")?;
        let stored_count = u32::from_le_bytes(verify.data[8..12].try_into().unwrap());
        ensure!(
            stored_count == entry_count,
            "verification failed: expected {} entries, stored {}",
            entry_count,
            stored_count
        );

        Ok(BuildStats { entry_count })
    }
}

/// Parse variable-length type-tagged refs from a byte slice.
fn parse_refs(data: &[u8]) -> Vec<IndexRef> {
    let mut refs = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        match IndexRef::from_bytes(data, pos) {
            Some((r, consumed)) => {
                refs.push(r);
                pos += consumed;
            }
            None => break,
        }
    }
    refs
}

fn hex_to_bytes(hex: &str) -> Result<[u8; 32]> {
    ensure!(
        hex.len() == 64,
        "expected 64-char hex, got {} chars",
        hex.len()
    );
    let bytes = hex::decode(hex).context("hex decode failed")?;
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("wrong byte count"))?;
    Ok(arr)
}

fn read_u64_le(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_index_ref_manifest_shard_roundtrip() {
        let r = IndexRef::DocumentShard { prefix: 0xab };
        let bytes = r.to_bytes();
        assert_eq!(bytes, vec![0x00, 0xab]);
        let (parsed, consumed) = IndexRef::from_bytes(&bytes, 0).unwrap();
        assert_eq!(consumed, 2);
        assert_eq!(parsed, r);
    }

    #[test]
    fn test_index_ref_token_range_roundtrip() {
        let r = IndexRef::TokenIndexRange {
            tokenizer: "spm".to_string(),
            file_number: 3,
            byte_offset: 0x1000,
            length: 0x2000,
        };
        let bytes = r.to_bytes();
        let (parsed, consumed) = IndexRef::from_bytes(&bytes, 0).unwrap();
        assert_eq!(consumed, bytes.len());
        assert_eq!(parsed, r);
    }

    #[test]
    fn test_parse_refs_mixed() {
        let r1 = IndexRef::DocumentShard { prefix: 0x5a };
        let r2 = IndexRef::TokenIndexRange {
            tokenizer: "bpe-test".to_string(),
            file_number: 10,
            byte_offset: 999,
            length: 42,
        };
        let r3 = IndexRef::DocumentShard { prefix: 0x00 };
        let mut buf = Vec::new();
        buf.extend_from_slice(&r1.to_bytes());
        buf.extend_from_slice(&r2.to_bytes());
        buf.extend_from_slice(&r3.to_bytes());

        let parsed = parse_refs(&buf);
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0], r1);
        assert_eq!(parsed[1], r2);
        assert_eq!(parsed[2], r3);
    }

    #[test]
    fn test_obidxf02_token_ref_roundtrip() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let ob_dir = tmp_dir.path();
        std::fs::create_dir_all(ob_dir.join(".ob")).unwrap();

        let id1_hex = "aa".repeat(32);
        let id2_hex = "bb".repeat(32);

        let id1_bytes = hex_to_bytes(&id1_hex).unwrap();
        let id2_bytes = hex_to_bytes(&id2_hex).unwrap();

        let entries = vec![
            (
                id1_bytes,
                vec![
                    IndexRef::DocumentShard { prefix: 0x0f },
                    IndexRef::TokenIndexRange {
                        tokenizer: "sentencepiece".to_string(),
                        file_number: 7,
                        byte_offset: 12345,
                        length: 67890,
                    },
                ],
            ),
            (
                id2_bytes,
                vec![
                    IndexRef::TokenIndexRange {
                        tokenizer: "tiktoken".to_string(),
                        file_number: 1,
                        byte_offset: 0,
                        length: 100,
                    },
                    IndexRef::DocumentShard { prefix: 0xff },
                ],
            ),
        ];

        let stats = BinaryIndex::build(ob_dir, entries).unwrap();
        assert_eq!(stats.entry_count, 2);

        let idx = BinaryIndex::open(ob_dir).unwrap().unwrap();

        let refs1 = idx.lookup(&id1_hex);
        assert_eq!(refs1.len(), 2);
        assert_eq!(refs1[0], IndexRef::DocumentShard { prefix: 0x0f });
        assert_eq!(
            refs1[1],
            IndexRef::TokenIndexRange {
                tokenizer: "sentencepiece".to_string(),
                file_number: 7,
                byte_offset: 12345,
                length: 67890,
            }
        );

        let refs2 = idx.lookup(&id2_hex);
        assert_eq!(refs2.len(), 2);
        assert_eq!(
            refs2[0],
            IndexRef::TokenIndexRange {
                tokenizer: "tiktoken".to_string(),
                file_number: 1,
                byte_offset: 0,
                length: 100,
            }
        );
        assert_eq!(refs2[1], IndexRef::DocumentShard { prefix: 0xff });

        let refs3 = idx.lookup(&"cc".repeat(32));
        assert!(refs3.is_empty());
    }

    #[test]
    fn test_obidxf01_rejected() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let ob_dir = tmp_dir.path();
        std::fs::create_dir_all(ob_dir.join(".ob")).unwrap();

        let out_path = ob_dir.join(".ob").join("index.bin");
        let mut file = std::fs::File::create(&out_path).unwrap();
        file.write_all(b"OBIDXF01").unwrap();
        file.write_all(&0u32.to_le_bytes()).unwrap();
        file.write_all(&0u32.to_le_bytes()).unwrap();
        file.write_all(&0u64.to_le_bytes()).unwrap();
        file.write_all(&[0u8; 256 * 8]).unwrap();
        drop(file);

        let result = BinaryIndex::open(ob_dir);
        assert!(result.is_err());
        let err_msg = match result {
            Ok(_) => panic!("expected error for OBIDXF01 file"),
            Err(e) => e.to_string(),
        };
        assert!(
            err_msg.contains("OBIDXF01"),
            "expected OBIDXF01 in error, got: {}",
            err_msg
        );
        assert!(
            err_msg.contains("ob index build"),
            "expected rebuild suggestion, got: {}",
            err_msg
        );
    }
}
