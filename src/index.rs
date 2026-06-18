use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::Path;

pub struct IndexStats {
    pub authors: usize,
    pub sections: usize,
    pub total: usize,
    pub token_index_entries: usize,
}

pub fn build_index(ob_dir: &Path) -> Result<IndexStats> {
    let mut section_to_manifest_buckets: HashMap<String, HashSet<String>> = HashMap::new();
    let manifest_dir = ob_dir.join(".ob").join("document-index");
    if manifest_dir.is_dir() {
        for entry in std::fs::read_dir(&manifest_dir)? {
            let entry = entry?;
            if !entry.path().is_file() {
                continue;
            }
            for rec in crate::storage::jsonl_read(&entry.path())? {
                let sources = match rec.get("sources").and_then(|v| v.as_array()) {
                    Some(s) => s,
                    None => continue,
                };
                let line_hash = match rec.get("line_hash").and_then(|v| v.as_str()) {
                    Some(lh) if !lh.is_empty() => lh,
                    _ => continue,
                };
                let manifest_bucket = &line_hash[..2];
                for src in sources {
                    if let Some(section_hash) = src.as_str() {
                        section_to_manifest_buckets
                            .entry(section_hash.to_string())
                            .or_default()
                            .insert(manifest_bucket.to_string());
                    }
                }
            }
        }
    }

    let mut author_to_section_buckets: HashMap<String, HashSet<String>> = HashMap::new();
    let mut section_records: HashMap<String, Vec<String>> = HashMap::new();
    let mut author_ids: HashSet<String> = HashSet::new();
    let mut section_ids: HashSet<String> = HashSet::new();

    let section_dir = ob_dir.join(".ob").join("sections");
    if section_dir.is_dir() {
        for entry in std::fs::read_dir(&section_dir)? {
            let entry = entry?;
            if !entry.path().is_file() {
                continue;
            }
            for rec in crate::storage::jsonl_read(&entry.path())? {
                let section_hash = match rec.get("section_hash").and_then(|v| v.as_str()) {
                    Some(sh) if !sh.is_empty() => sh,
                    _ => continue,
                };
                let section_bucket = &section_hash[..2];
                let manifest_refs = match section_to_manifest_buckets.get(section_hash) {
                    Some(set) => {
                        let mut v: Vec<&str> = set.iter().map(|s| s.as_str()).collect();
                        v.sort();
                        v.into_iter().map(|s| s.to_string()).collect()
                    }
                    None => Vec::new(),
                };
                section_records.insert(section_hash.to_string(), manifest_refs);
                section_ids.insert(section_hash.to_string());

                if let Some(authors) = rec.get("authors").and_then(|v| v.as_array()) {
                    for aid in authors {
                        if let Some(author_id) = aid.as_str() {
                            author_to_section_buckets
                                .entry(author_id.to_string())
                                .or_default()
                                .insert(section_bucket.to_string());
                            author_ids.insert(author_id.to_string());
                        }
                    }
                }
            }
        }
    }

    let mut bucket_records: HashMap<String, Vec<serde_json::Value>> = HashMap::new();

    for author_id in &author_ids {
        let bucket = &author_id[..2];
        let mut refs: Vec<&str> = author_to_section_buckets
            .get(author_id)
            .map(|s| s.iter().map(|x| x.as_str()).collect())
            .unwrap_or_default();
        refs.sort();
        bucket_records
            .entry(bucket.to_string())
            .or_default()
            .push(serde_json::json!({"id": author_id, "refs": refs}));
    }

    for (section_hash, refs) in &section_records {
        let bucket = &section_hash[..2];
        bucket_records
            .entry(bucket.to_string())
            .or_default()
            .push(serde_json::json!({"id": section_hash, "refs": refs}));
    }

    let index_dir = ob_dir.join(".ob").join("index");
    if index_dir.is_dir() {
        for entry in std::fs::read_dir(&index_dir)? {
            let entry = entry?;
            if entry.path().is_file() {
                std::fs::remove_file(entry.path())?;
            }
        }
    }
    std::fs::create_dir_all(&index_dir)?;

    for (bucket, records) in &bucket_records {
        let path = index_dir.join(bucket);
        let mut file = std::fs::File::create(&path)?;
        for rec in records {
            writeln!(file, "{}", serde_json::to_string(rec)?)?;
        }
    }

    let mut token_index_entries = 0usize;
    {
        let mut bin_entries: Vec<([u8; 32], Vec<crate::binary_index::IndexRef>)> = Vec::new();
        let index_dir_for_scan = ob_dir.join(".ob").join("index");
        if index_dir_for_scan.is_dir() {
            for entry in std::fs::read_dir(&index_dir_for_scan)? {
                let entry = entry?;
                let path = entry.path();
                if !path.is_file() || path.extension().is_some() {
                    continue;
                }
                for rec in crate::storage::jsonl_read(&path)? {
                    let id_hex = match rec.get("id").and_then(|v| v.as_str()) {
                        Some(id) => id,
                        None => continue,
                    };
                    let id_bytes: [u8; 32] = match hex::decode(id_hex) {
                        Ok(b) => match b.try_into() {
                            Ok(a) => a,
                            Err(_) => continue,
                        },
                        Err(_) => continue,
                    };
                    let refs: Vec<crate::binary_index::IndexRef> = match rec
                        .get("refs")
                        .and_then(|v| v.as_array())
                    {
                        Some(arr) => arr
                            .iter()
                            .filter_map(|v| v.as_str())
                            .filter_map(|s| u8::from_str_radix(s, 16).ok())
                            .map(|prefix| crate::binary_index::IndexRef::DocumentShard { prefix })
                            .collect(),
                        None => Vec::new(),
                    };
                    bin_entries.push((id_bytes, refs));
                }
            }
        }
        let mut entry_map: HashMap<[u8; 32], usize> = HashMap::new();
        for (i, (id, _)) in bin_entries.iter().enumerate() {
            entry_map.insert(*id, i);
        }

        let tokenizers = crate::token_index::list_tokenizers(ob_dir)?;
        for tokenizer in &tokenizers {
            let merged_dir = ob_dir
                .join(".ob")
                .join(format!("token-index.{}", tokenizer));
            if !merged_dir.is_dir() {
                continue;
            }

            let mut files: Vec<(u64, std::path::PathBuf)> = Vec::new();
            for f in std::fs::read_dir(&merged_dir)? {
                let f = f?;
                let name = f.file_name().to_string_lossy().to_string();
                if let Ok(num) = name.parse::<u64>() {
                    files.push((num, f.path()));
                }
            }
            files.sort_by_key(|(num, _)| *num);

            for (file_number, path) in &files {
                let mmap = match crate::mmap_lines::mmap_file(&path)? {
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

                    let rec: serde_json::Value = match serde_json::from_slice(line) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    let sources = match rec.get("sources").and_then(|v| v.as_array()) {
                        Some(s) => s,
                        None => continue,
                    };

                    token_index_entries += 1;
                    let byte_offset = line_start as u32;
                    let length = (line_end - line_start) as u32;
                    let tir = crate::binary_index::IndexRef::TokenIndexRange {
                        tokenizer: tokenizer.clone(),
                        file_number: *file_number as u16,
                        byte_offset,
                        length,
                    };

                    for src in sources {
                        let src_hex = match src.as_str() {
                            Some(s) => s,
                            None => continue,
                        };
                        let src_bytes: [u8; 32] = match hex::decode(src_hex) {
                            Ok(b) => match b.try_into() {
                                Ok(a) => a,
                                Err(_) => continue,
                            },
                            Err(_) => continue,
                        };
                        if let Some(&idx) = entry_map.get(&src_bytes) {
                            bin_entries[idx].1.push(tir.clone());
                        } else {
                            entry_map.insert(src_bytes, bin_entries.len());
                            bin_entries.push((src_bytes, vec![tir.clone()]));
                        }
                    }
                }
            }
        }

        crate::binary_index::BinaryIndex::build(ob_dir, bin_entries)?;
    }

    let tokenizers = crate::token_index::list_tokenizers(ob_dir)?;
    for tokenizer in &tokenizers {
        let _ = crate::token_index::build_binary_index(ob_dir, tokenizer);
    }

    for entry in std::fs::read_dir(ob_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().map_or(false, |e| e == "jsonl") {
            crate::binary_index::BinaryIndex::append_line_offsets(ob_dir, &path)?;
            break;
        }
    }

    Ok(IndexStats {
        authors: author_ids.len(),
        sections: section_ids.len(),
        total: author_ids.len() + section_ids.len(),
        token_index_entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_test_ob_dir(
        tmp: &tempfile::TempDir,
        section_hashes: &[&str],
        token_index_entries: &[(&str, u64, &[&str])],
    ) -> std::path::PathBuf {
        let ob_dir = tmp.path().to_path_buf();
        let ob = ob_dir.join(".ob");
        std::fs::create_dir_all(&ob).unwrap();

        let manifest_dir = ob.join("document-index");
        std::fs::create_dir_all(&manifest_dir).unwrap();
        let manifest_path = manifest_dir.join("00");
        let mut f = std::fs::File::create(&manifest_path).unwrap();
        for (i, sh) in section_hashes.iter().enumerate() {
            let line_hash = format!("{:02x}{}", i % 256, "a".repeat(62));
            writeln!(
                f,
                r#"{{"line_hash":"{}","sources":["{}"]}}"#,
                line_hash, sh
            )
            .unwrap();
        }

        let section_dir = ob.join("sections");
        std::fs::create_dir_all(&section_dir).unwrap();
        let section_path = section_dir.join("00");
        let mut f = std::fs::File::create(&section_path).unwrap();
        for sh in section_hashes {
            writeln!(
                f,
                r#"{{"section_hash":"{}","authors":["author1"]}}"#,
                sh
            )
            .unwrap();
        }

        for (tokenizer, file_num, sources) in token_index_entries {
            let ti_dir = ob.join(format!("token-index.{}", tokenizer));
            std::fs::create_dir_all(&ti_dir).unwrap();
            let ti_path = ti_dir.join(format!("{:03}", file_num));
            let srcs: Vec<String> = sources.iter().map(|s| format!(r#""{}""#, s)).collect();
            let srcs_json = format!("[{}]", srcs.join(","));
            let line = format!(
                r#"{{"token_count":5,"sources":{},"tokenizer":"{}","revoked":false}}"#,
                srcs_json, tokenizer
            );
            std::fs::write(&ti_path, format!("{}\n", line)).unwrap();
        }

        ob_dir
    }

    #[test]
    fn test_token_index_refs_in_binary_index() {
        let tmp = tempfile::tempdir().unwrap();

        let section_hash: String = "aa".repeat(32);
        let orphan_hash: String = "bb".repeat(32);

        let ob_dir = setup_test_ob_dir(
            &tmp,
            &[&section_hash],
            &[("gpt2", 0, &[&section_hash, &orphan_hash])],
        );

        let stats = build_index(&ob_dir).unwrap();
        assert_eq!(stats.token_index_entries, 1);

        let idx = crate::binary_index::BinaryIndex::open(&ob_dir)
            .unwrap()
            .unwrap();

        let refs = idx.lookup(&section_hash);
        assert!(
            refs.iter().any(|r| matches!(
                r,
                crate::binary_index::IndexRef::TokenIndexRange { tokenizer, .. }
                if tokenizer == "gpt2"
            )),
            "expected TokenIndexRange ref for section_hash"
        );

        let orphan_refs = idx.lookup(&orphan_hash);
        assert!(
            orphan_refs.iter().any(|r| matches!(
                r,
                crate::binary_index::IndexRef::TokenIndexRange { tokenizer, file_number, .. }
                if tokenizer == "gpt2" && *file_number == 0
            )),
            "expected TokenIndexRange ref for orphan_hash"
        );
    }

    #[test]
    fn test_token_index_byte_offsets_accurate() {
        let tmp = tempfile::tempdir().unwrap();

        let sh1: String = "11".repeat(32);
        let sh2: String = "22".repeat(32);

        let ob_dir = setup_test_ob_dir(
            &tmp,
            &[&sh1],
            &[("bpe", 0, &[&sh1]), ("bpe", 1, &[&sh2])],
        );

        let stats = build_index(&ob_dir).unwrap();
        assert_eq!(stats.token_index_entries, 2);

        let idx = crate::binary_index::BinaryIndex::open(&ob_dir)
            .unwrap()
            .unwrap();

        let refs1 = idx.lookup(&sh1);
        let tir1 = refs1.iter().find_map(|r| match r {
            crate::binary_index::IndexRef::TokenIndexRange { file_number, byte_offset, length, .. }
            if *file_number == 0 => Some((*byte_offset, *length)),
            _ => None,
        });
        assert!(tir1.is_some(), "expected TokenIndexRange ref in file 0 for sh1");
        let (off, len) = tir1.unwrap();
        let mmap = crate::mmap_lines::mmap_file(
            &ob_dir.join(".ob").join("token-index.bpe").join("000"),
        )
        .unwrap()
        .unwrap();
        let line_data = &mmap[off as usize..(off + len) as usize];
        let rec: serde_json::Value = serde_json::from_slice(line_data).unwrap();
        assert_eq!(rec["sources"][0].as_str().unwrap(), sh1);

        let refs2 = idx.lookup(&sh2);
        let tir2 = refs2.iter().find_map(|r| match r {
            crate::binary_index::IndexRef::TokenIndexRange { file_number, byte_offset, length, .. }
            if *file_number == 1 => Some((*byte_offset, *length)),
            _ => None,
        });
        assert!(tir2.is_some(), "expected TokenIndexRange ref in file 1 for sh2");
        let (off2, len2) = tir2.unwrap();
        let mmap2 = crate::mmap_lines::mmap_file(
            &ob_dir.join(".ob").join("token-index.bpe").join("001"),
        )
        .unwrap()
        .unwrap();
        let line_data2 = &mmap2[off2 as usize..(off2 + len2) as usize];
        let rec2: serde_json::Value = serde_json::from_slice(line_data2).unwrap();
        assert_eq!(rec2["sources"][0].as_str().unwrap(), sh2);
    }

    #[test]
    fn test_no_token_index_data() {
        let tmp = tempfile::tempdir().unwrap();

        let section_hash: String = "cc".repeat(32);
        let ob_dir = setup_test_ob_dir(&tmp, &[&section_hash], &[]);

        let stats = build_index(&ob_dir).unwrap();
        assert_eq!(stats.token_index_entries, 0);
    }
}
