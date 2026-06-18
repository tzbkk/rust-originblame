use serde::Serialize;
use sha2::{Digest, Sha256};

pub fn compute_hash<T: Serialize>(data: &T) -> String {
    let json = serde_json::to_string(data).unwrap();
    let mut hasher = Sha256::new();
    hasher.update(json.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_deterministic() {
        let data = serde_json::json!({"text": "hello", "lang": "en"});
        let h1 = compute_hash(&data);
        let h2 = compute_hash(&data);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64);
    }

    #[test]
    fn hash_key_order_independent() {
        let h1 = compute_hash(&serde_json::json!({"a": 1, "b": 2}));
        let h2 = compute_hash(&serde_json::json!({"b": 2, "a": 1}));
        assert_eq!(h1, h2);
    }
}
