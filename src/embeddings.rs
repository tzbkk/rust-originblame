use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

pub fn shard_append_embedding(
    ob_dir: &Path,
    model: &str,
    line_hash: &str,
    embedding: &[f64],
) -> Result<()> {
    let layer = format!("embeddings.{}", model);
    let shard = crate::storage::shard_path(ob_dir, &layer, line_hash);
    let record = serde_json::json!({
        "line_hash": line_hash,
        "embedding": embedding
    });
    let dir = shard.parent().unwrap();
    std::fs::create_dir_all(dir)?;
    crate::storage::jsonl_append(&shard, &record)?;
    Ok(())
}

pub fn read_all_embeddings(ob_dir: &Path, model: &str) -> Result<HashMap<String, Vec<f64>>> {
    let layer = format!("embeddings.{}", model);
    let records = crate::storage::shard_iterate_all(ob_dir, &layer)?;
    let mut map = HashMap::new();
    for rec in records {
        if let (Some(lh), Some(emb)) = (
            rec.get("line_hash").and_then(|v| v.as_str()),
            rec.get("embedding").and_then(|v| v.as_array()),
        ) {
            let vec: Vec<f64> = emb.iter().filter_map(|v| v.as_f64()).collect();
            map.insert(lh.to_string(), vec);
        }
    }
    Ok(map)
}
