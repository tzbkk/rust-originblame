import json
import logging
import math
from pathlib import Path
from typing import Callable

import pytest

from ob.exceptions import OBStorageError
from ob.storage import shard_append, shard_read, jsonl_read
from ob.util import compute_hash
from ob_util.reconcile import ReconcileResult, reconcile, cosine_similarity


# ---------------------------------------------------------------------------
# Helpers / fixtures
# ---------------------------------------------------------------------------


def _init_ob_dir(base: Path) -> Path:
    from ob.api import init as ob_init

    ob_init(ob_dir=base, force=True)
    return base


def _write_manifest_entry(
    ob_dir: Path,
    line_hash: str,
    file: str,
    sources: list[str],
    source_type: str = "track",
):
    shard_append(
        ob_dir,
        "document-index",
        line_hash,
        {
            "line_hash": line_hash,
            "file": file,
            "sources": sources,
            "source_type": source_type,
            "revoked": False,
        },
    )


def _write_embedding(ob_dir: Path, model: str, line_hash: str, embedding: list[float]):
    shard_append(
        ob_dir,
        f"embeddings.{model}",
        line_hash,
        {
            "line_hash": line_hash,
            "embedding": embedding,
        },
    )


def _make_encode_fn(
    mapping: dict[str, list[float]],
) -> Callable[[list[str]], list[list[float]]]:
    """Create mock encode function. mapping: {text: embedding}"""
    dim = len(next(iter(mapping.values()))) if mapping else 4

    def encode(texts: list[str]) -> list[list[float]]:
        return [mapping.get(t, [0.0] * dim) for t in texts]

    return encode


# ---------------------------------------------------------------------------
# cosine_similarity
# ---------------------------------------------------------------------------


class TestCosineSimilarity:
    def test_identical_vectors(self):
        assert cosine_similarity([1, 0, 0], [1, 0, 0]) == 1.0

    def test_orthogonal_vectors(self):
        assert cosine_similarity([1, 0, 0], [0, 1, 0]) == 0.0

    def test_opposite_vectors(self):
        assert cosine_similarity([1, 0, 0], [-1, 0, 0]) == -1.0

    def test_scaled_vectors(self):
        assert cosine_similarity([1, 2, 3], [2, 4, 6]) == 1.0

    def test_zero_vector(self):
        assert cosine_similarity([0, 0, 0], [1, 2, 3]) == 0.0

    def test_dimension_mismatch(self):
        with pytest.raises(ValueError):
            cosine_similarity([1, 2], [1, 2, 3])

    def test_known_value(self):
        # (4+10+18) / (sqrt(14)*sqrt(77))
        dot = 4 + 10 + 18  # 32
        mag_a = math.sqrt(14)
        mag_b = math.sqrt(77)
        expected = dot / (mag_a * mag_b)
        result = cosine_similarity([1, 2, 3], [4, 5, 6])
        assert abs(result - expected) < 1e-10


# ---------------------------------------------------------------------------
# ReconcileResult dataclass
# ---------------------------------------------------------------------------


class TestReconcileResult:
    def test_default_values(self):
        r = ReconcileResult()
        assert r.hash_matched == 0
        assert r.semantic_matched == 0
        assert r.new_lines == 0
        assert r.orphans == 0
        assert r.errors == 0
        assert r.orphan_hashes == []
        assert r.duration_ms == 0.0

    def test_custom_values(self):
        r = ReconcileResult(
            hash_matched=5,
            semantic_matched=2,
            new_lines=3,
            orphans=1,
            errors=1,
            orphan_hashes=["abc", "def"],
            duration_ms=42.5,
        )
        assert r.hash_matched == 5
        assert r.semantic_matched == 2
        assert r.new_lines == 3
        assert r.orphans == 1
        assert r.errors == 1
        assert r.orphan_hashes == ["abc", "def"]
        assert r.duration_ms == 42.5

    def test_orphan_hashes_default_empty(self):
        r = ReconcileResult()
        assert r.orphan_hashes is not None
        assert r.orphan_hashes == []


# ---------------------------------------------------------------------------
# Pass 1 — hash matching
# ---------------------------------------------------------------------------


class TestHashMatch:
    def test_tracked_line_hash_matched(self, tmp_path):
        ob_dir = _init_ob_dir(tmp_path)
        line = "hello world"
        line_hash = compute_hash(line)
        _write_manifest_entry(ob_dir, line_hash, "data.txt", ["section1"])

        data_file = tmp_path / "data.txt"
        data_file.write_text(line + "\n")

        result = reconcile(str(data_file), ob_dir=ob_dir)
        assert result.hash_matched == 1
        assert result.new_lines == 0

    def test_json_dict_line_hash_matched(self, tmp_path):
        ob_dir = _init_ob_dir(tmp_path)
        data = {"text": "hello", "source": "wiki"}
        line_hash = compute_hash(data)
        _write_manifest_entry(ob_dir, line_hash, "data.jsonl", ["s1"])

        data_file = tmp_path / "data.jsonl"
        data_file.write_text(json.dumps(data) + "\n")

        result = reconcile(str(data_file), ob_dir=ob_dir)
        assert result.hash_matched == 1
        assert result.new_lines == 0

    def test_multiple_lines_all_matched(self, tmp_path):
        ob_dir = _init_ob_dir(tmp_path)
        lines = ["line one", "line two", "line three"]
        for line in lines:
            h = compute_hash(line)
            _write_manifest_entry(ob_dir, h, "data.txt", ["s1"])

        data_file = tmp_path / "data.txt"
        data_file.write_text("\n".join(lines) + "\n")

        result = reconcile(str(data_file), ob_dir=ob_dir)
        assert result.hash_matched == 3
        assert result.new_lines == 0

    def test_hash_match_file_specific(self, tmp_path):
        """Same hash in a different file should NOT match."""
        ob_dir = _init_ob_dir(tmp_path)
        line = "shared content"
        line_hash = compute_hash(line)
        # Write manifest entry for "other.txt", not "data.txt"
        _write_manifest_entry(ob_dir, line_hash, "other.txt", ["s1"])

        data_file = tmp_path / "data.txt"
        data_file.write_text(line + "\n")

        result = reconcile(str(data_file), ob_dir=ob_dir)
        assert result.hash_matched == 0
        assert result.new_lines == 1

    def test_hash_match_ignores_revoked(self, tmp_path):
        """Revoked manifest entries should not count as matches."""
        ob_dir = _init_ob_dir(tmp_path)
        line = "revoked content"
        line_hash = compute_hash(line)
        shard_append(
            ob_dir,
            "document-index",
            line_hash,
            {
                "line_hash": line_hash,
                "file": "data.txt",
                "sources": ["s1"],
                "source_type": "track",
                "revoked": True,
            },
        )

        data_file = tmp_path / "data.txt"
        data_file.write_text(line + "\n")

        result = reconcile(str(data_file), ob_dir=ob_dir)
        assert result.hash_matched == 0
        assert result.new_lines == 1

    def test_different_sources_separate_records(self, tmp_path):
        """Same hash+file with different sources — both records preserved."""
        ob_dir = _init_ob_dir(tmp_path)
        line = "shared line"
        line_hash = compute_hash(line)
        _write_manifest_entry(ob_dir, line_hash, "data.txt", ["s1"])
        _write_manifest_entry(ob_dir, line_hash, "data.txt", ["s2"])

        data_file = tmp_path / "data.txt"
        data_file.write_text(line + "\n")

        result = reconcile(str(data_file), ob_dir=ob_dir)
        assert result.hash_matched == 1

        # Both records should still exist
        records = shard_read(ob_dir, "document-index", line_hash)
        source_lists = {
            tuple(sorted(r["sources"])) for r in records if r.get("file") == "data.txt"
        }
        assert ("s1",) in source_lists
        assert ("s2",) in source_lists


# ---------------------------------------------------------------------------
# New lines (unmatched in pass 1, no embedding match)
# ---------------------------------------------------------------------------


class TestNewLines:
    def test_untracked_line_is_new(self, tmp_path):
        ob_dir = _init_ob_dir(tmp_path)
        data_file = tmp_path / "data.txt"
        data_file.write_text("brand new line\n")

        result = reconcile(str(data_file), ob_dir=ob_dir)
        assert result.hash_matched == 0
        assert result.new_lines == 1

    def test_mix_of_tracked_and_new(self, tmp_path):
        ob_dir = _init_ob_dir(tmp_path)
        tracked = "existing line"
        h = compute_hash(tracked)
        _write_manifest_entry(ob_dir, h, "data.txt", ["s1"])

        data_file = tmp_path / "data.txt"
        data_file.write_text(tracked + "\nbrand new line\n")

        result = reconcile(str(data_file), ob_dir=ob_dir)
        assert result.hash_matched == 1
        assert result.new_lines == 1

    def test_empty_lines_skipped(self, tmp_path):
        ob_dir = _init_ob_dir(tmp_path)
        h = compute_hash("real")
        _write_manifest_entry(ob_dir, h, "data.txt", ["s1"])

        data_file = tmp_path / "data.txt"
        data_file.write_text("real\n\n   \n")

        result = reconcile(str(data_file), ob_dir=ob_dir)
        assert result.hash_matched == 1
        assert result.new_lines == 0

    def test_empty_file_zero_totals(self, tmp_path):
        ob_dir = _init_ob_dir(tmp_path)
        data_file = tmp_path / "data.txt"
        data_file.write_text("")

        result = reconcile(str(data_file), ob_dir=ob_dir)
        assert result.hash_matched == 0
        assert result.new_lines == 0
        assert result.errors == 0


# ---------------------------------------------------------------------------
# Validation / error handling
# ---------------------------------------------------------------------------


class TestValidation:
    def test_unmerged_pid_files_raises_error(self, tmp_path):
        ob_dir = _init_ob_dir(tmp_path)
        # Create an unmerged PID file
        (ob_dir / ".ob" / "docidx.12345").write_text(
            json.dumps({"line_hash": "a" * 64, "file": "x"}) + "\n"
        )

        data_file = tmp_path / "data.txt"
        data_file.write_text("some line\n")

        with pytest.raises(OBStorageError):
            reconcile(str(data_file), ob_dir=ob_dir)


# ---------------------------------------------------------------------------
# Pass 2 — embedding / semantic matching (mocked, no ML deps)
# ---------------------------------------------------------------------------


class TestEmbeddingMatch:
    def test_similar_content_semantic_match(self, tmp_path):
        ob_dir = _init_ob_dir(tmp_path)
        file = "data.txt"

        old_text = "the quick brown fox"
        old_hash = compute_hash(old_text)
        _write_manifest_entry(ob_dir, old_hash, file, ["s1"])
        _write_embedding(ob_dir, "test-model", old_hash, [1.0, 0.0, 0.0, 0.0])

        new_text = "the quick brown cat"
        new_hash = compute_hash(new_text)
        encode_fn = _make_encode_fn({new_text: [0.99, 0.01, 0.0, 0.0]})

        data_file = tmp_path / "data.txt"
        data_file.write_text(new_text + "\n")

        result = reconcile(
            str(data_file),
            model="test-model",
            threshold=0.85,
            ob_dir=ob_dir,
            _encode_fn=encode_fn,
        )

        assert result.hash_matched == 0
        assert result.semantic_matched == 1
        assert result.new_lines == 0

        # New manifest record created with source_type="reconcile"
        new_records = shard_read(ob_dir, "document-index", new_hash)
        matched = [
            r
            for r in new_records
            if r.get("line_hash") == new_hash and r.get("file") == file
        ]
        assert len(matched) == 1
        assert matched[0]["source_type"] == "reconcile"
        assert matched[0]["sources"] == ["s1"]

        # Old manifest record removed for this file
        old_records = shard_read(ob_dir, "document-index", old_hash)
        old_for_file = [
            r
            for r in old_records
            if r.get("line_hash") == old_hash and r.get("file") == file
        ]
        assert len(old_for_file) == 0

    def test_below_threshold_no_match(self, tmp_path):
        ob_dir = _init_ob_dir(tmp_path)
        file = "data.txt"

        old_text = "alpha content"
        old_hash = compute_hash(old_text)
        _write_manifest_entry(ob_dir, old_hash, file, ["s1"])
        _write_embedding(ob_dir, "test-model", old_hash, [1.0, 0.0, 0.0, 0.0])

        new_text = "zeta content"
        encode_fn = _make_encode_fn({new_text: [0.0, 1.0, 0.0, 0.0]})

        data_file = tmp_path / "data.txt"
        data_file.write_text(new_text + "\n")

        result = reconcile(
            str(data_file),
            model="test-model",
            threshold=0.85,
            ob_dir=ob_dir,
            _encode_fn=encode_fn,
        )

        assert result.semantic_matched == 0
        assert result.new_lines == 1

    def test_exactly_at_threshold_matches(self, tmp_path):
        ob_dir = _init_ob_dir(tmp_path)
        file = "data.txt"

        old_text = "original text"
        old_hash = compute_hash(old_text)
        _write_manifest_entry(ob_dir, old_hash, file, ["s1"])
        # cos([1,0,0,0], [0.85, sqrt(0.2775), 0, 0]) = 0.85 exactly
        _write_embedding(ob_dir, "test-model", old_hash, [1.0, 0.0, 0.0, 0.0])

        new_text = "modified text"
        b_component = math.sqrt(0.2775)
        encode_fn = _make_encode_fn({new_text: [0.85, b_component, 0.0, 0.0]})

        data_file = tmp_path / "data.txt"
        data_file.write_text(new_text + "\n")

        result = reconcile(
            str(data_file),
            model="test-model",
            threshold=0.85,
            ob_dir=ob_dir,
            _encode_fn=encode_fn,
        )

        assert result.semantic_matched == 1
        assert result.new_lines == 0

    def test_embedding_match_inherits_sources(self, tmp_path):
        ob_dir = _init_ob_dir(tmp_path)
        file = "data.txt"

        old_text = "source test"
        old_hash = compute_hash(old_text)
        _write_manifest_entry(ob_dir, old_hash, file, ["s1", "s2"])
        _write_embedding(ob_dir, "test-model", old_hash, [1.0, 0.0, 0.0, 0.0])

        new_text = "source test modified"
        new_hash = compute_hash(new_text)
        encode_fn = _make_encode_fn({new_text: [0.99, 0.01, 0.0, 0.0]})

        data_file = tmp_path / "data.txt"
        data_file.write_text(new_text + "\n")

        result = reconcile(
            str(data_file),
            model="test-model",
            threshold=0.85,
            ob_dir=ob_dir,
            _encode_fn=encode_fn,
        )

        assert result.semantic_matched == 1

        new_records = shard_read(ob_dir, "document-index", new_hash)
        matched = [
            r
            for r in new_records
            if r.get("line_hash") == new_hash and r.get("file") == file
        ]
        assert len(matched) == 1
        assert set(matched[0]["sources"]) == {"s1", "s2"}

    def test_embedding_match_stores_new_embedding(self, tmp_path):
        ob_dir = _init_ob_dir(tmp_path)
        file = "data.txt"
        model = "test-model"

        old_text = "store embedding test"
        old_hash = compute_hash(old_text)
        _write_manifest_entry(ob_dir, old_hash, file, ["s1"])
        _write_embedding(ob_dir, model, old_hash, [1.0, 0.0, 0.0, 0.0])

        new_text = "store embedding modified"
        new_hash = compute_hash(new_text)
        new_emb = [0.95, 0.05, 0.0, 0.0]
        encode_fn = _make_encode_fn({new_text: new_emb})

        data_file = tmp_path / "data.txt"
        data_file.write_text(new_text + "\n")

        result = reconcile(
            str(data_file),
            model=model,
            threshold=0.85,
            ob_dir=ob_dir,
            _encode_fn=encode_fn,
        )

        assert result.semantic_matched == 1

        # Verify new embedding stored
        emb_records = shard_read(ob_dir, f"embeddings.{model}", new_hash)
        matched = [r for r in emb_records if r.get("line_hash") == new_hash]
        assert len(matched) == 1
        assert matched[0]["embedding"] == new_emb

    def test_multiple_embedding_matches(self, tmp_path):
        ob_dir = _init_ob_dir(tmp_path)
        file = "data.txt"

        old1 = "first original"
        old1_hash = compute_hash(old1)
        _write_manifest_entry(ob_dir, old1_hash, file, ["s1"])
        _write_embedding(ob_dir, "test-model", old1_hash, [1.0, 0.0, 0.0, 0.0])

        old2 = "second original"
        old2_hash = compute_hash(old2)
        _write_manifest_entry(ob_dir, old2_hash, file, ["s2"])
        _write_embedding(ob_dir, "test-model", old2_hash, [0.0, 1.0, 0.0, 0.0])

        new1 = "first modified"
        new2 = "second modified"
        encode_fn = _make_encode_fn(
            {
                new1: [0.99, 0.01, 0.0, 0.0],
                new2: [0.01, 0.99, 0.0, 0.0],
            }
        )

        data_file = tmp_path / "data.txt"
        data_file.write_text(new1 + "\n" + new2 + "\n")

        result = reconcile(
            str(data_file),
            model="test-model",
            threshold=0.85,
            ob_dir=ob_dir,
            _encode_fn=encode_fn,
        )

        assert result.semantic_matched == 2
        assert result.new_lines == 0

    def test_no_existing_embeddings_all_new(self, tmp_path):
        ob_dir = _init_ob_dir(tmp_path)
        file = "data.txt"

        old_text = "has manifest no embedding"
        old_hash = compute_hash(old_text)
        _write_manifest_entry(ob_dir, old_hash, file, ["s1"])
        # No embedding stored for this model

        new_text = "completely different"
        new_hash = compute_hash(new_text)
        new_emb = [0.3, 0.7, 0.0, 0.0]
        encode_fn = _make_encode_fn({new_text: new_emb})

        data_file = tmp_path / "data.txt"
        data_file.write_text(new_text + "\n")

        result = reconcile(
            str(data_file),
            model="test-model",
            threshold=0.85,
            ob_dir=ob_dir,
            _encode_fn=encode_fn,
        )

        # No embeddings to match against → new_lines
        assert result.semantic_matched == 0
        assert result.new_lines == 1
        assert result.orphans == 1

        # New embedding should still be stored for future use
        emb_records = shard_read(ob_dir, "embeddings.test-model", new_hash)
        matched = [r for r in emb_records if r.get("line_hash") == new_hash]
        assert len(matched) == 1
        assert matched[0]["embedding"] == new_emb

    def test_model_none_skips_pass2(self, tmp_path):
        ob_dir = _init_ob_dir(tmp_path)
        file = "data.txt"

        old_text = "original text"
        old_hash = compute_hash(old_text)
        _write_manifest_entry(ob_dir, old_hash, file, ["s1"])
        _write_embedding(ob_dir, "test-model", old_hash, [1.0, 0.0, 0.0, 0.0])

        new_text = "modified text"

        data_file = tmp_path / "data.txt"
        data_file.write_text(new_text + "\n")

        result = reconcile(str(data_file), model=None, ob_dir=ob_dir)

        assert result.semantic_matched == 0
        assert result.new_lines == 1


# ---------------------------------------------------------------------------
# Orphan detection (old manifest lines not matched by any data line)
# ---------------------------------------------------------------------------


class TestOrphanDetection:
    def test_deleted_lines_are_orphans(self, tmp_path):
        ob_dir = _init_ob_dir(tmp_path)
        file = "data.txt"

        line_a = "keep line A"
        line_b = "delete line B"
        line_c = "keep line C"

        hash_a = compute_hash(line_a)
        hash_b = compute_hash(line_b)
        hash_c = compute_hash(line_c)

        _write_manifest_entry(ob_dir, hash_a, file, ["s1"])
        _write_manifest_entry(ob_dir, hash_b, file, ["s2"])
        _write_manifest_entry(ob_dir, hash_c, file, ["s3"])

        # Data file missing line B
        data_file = tmp_path / "data.txt"
        data_file.write_text(line_a + "\n" + line_c + "\n")

        result = reconcile(str(data_file), ob_dir=ob_dir)

        assert result.orphans == 1
        assert hash_b in result.orphan_hashes

        b_records = shard_read(ob_dir, "document-index", hash_b)
        assert any(r.get("line_hash") == hash_b for r in b_records)
        assert any(r.get("orphan") is True for r in b_records)

    def test_no_orphans_when_all_matched(self, tmp_path):
        ob_dir = _init_ob_dir(tmp_path)
        file = "data.txt"

        for line in ["line one", "line two"]:
            _write_manifest_entry(ob_dir, compute_hash(line), file, ["s1"])

        data_file = tmp_path / "data.txt"
        data_file.write_text("line one\nline two\n")

        result = reconcile(str(data_file), ob_dir=ob_dir)

        assert result.orphans == 0
        assert result.orphan_hashes == []

    def test_embedding_matched_old_not_orphan(self, tmp_path):
        ob_dir = _init_ob_dir(tmp_path)
        file = "data.txt"

        line_a = "line A"
        line_b = "line B original"
        line_c = "line C"

        hash_a = compute_hash(line_a)
        hash_b = compute_hash(line_b)
        hash_c = compute_hash(line_c)

        _write_manifest_entry(ob_dir, hash_a, file, ["s1"])
        _write_manifest_entry(ob_dir, hash_b, file, ["s2"])
        _write_manifest_entry(ob_dir, hash_c, file, ["s3"])

        # Embedding for B so it can be semantically matched
        _write_embedding(ob_dir, "test-model", hash_b, [1.0, 0.0, 0.0, 0.0])

        # Data file: A + modified B (different text/hash, similar embedding) + C
        line_b_mod = "line B modified"
        encode_fn = _make_encode_fn({line_b_mod: [0.99, 0.01, 0.0, 0.0]})

        data_file = tmp_path / "data.txt"
        data_file.write_text(line_a + "\n" + line_b_mod + "\n" + line_c + "\n")

        result = reconcile(
            str(data_file),
            model="test-model",
            threshold=0.85,
            ob_dir=ob_dir,
            _encode_fn=encode_fn,
        )

        # A and C hash-matched, B embedding-matched → no orphans
        assert result.orphans == 0
        assert result.orphan_hashes == []


# ---------------------------------------------------------------------------
# Oplog integration
# ---------------------------------------------------------------------------


class TestOplogIntegration:
    def test_reconcile_logs_to_oplog(self, tmp_path):
        ob_dir = _init_ob_dir(tmp_path)
        data_file = tmp_path / "data.txt"
        data_file.write_text("some line\n")

        reconcile(str(data_file), ob_dir=ob_dir)

        log_entries = jsonl_read(ob_dir / ".ob" / "log")
        reconcile_entries = [e for e in log_entries if e.get("op") == "reconcile"]
        assert len(reconcile_entries) >= 1

    def test_oplog_contains_summary(self, tmp_path):
        ob_dir = _init_ob_dir(tmp_path)
        file = "data.txt"
        h = compute_hash("tracked line")
        _write_manifest_entry(ob_dir, h, file, ["s1"])

        data_file = tmp_path / "data.txt"
        data_file.write_text("tracked line\n")

        reconcile(
            str(data_file),
            model="test-model",
            threshold=0.9,
            ob_dir=ob_dir,
        )

        log_entries = jsonl_read(ob_dir / ".ob" / "log")
        reconcile_entries = [e for e in log_entries if e.get("op") == "reconcile"]
        assert len(reconcile_entries) >= 1

        detail = reconcile_entries[-1].get("detail", {})
        assert "file" in detail
        assert "hash_matched" in detail
        assert "model" in detail
        assert "threshold" in detail


class TestCrossFilePreservation:
    def test_embedding_match_preserves_other_file_record(self, tmp_path):
        ob_dir = _init_ob_dir(tmp_path)
        file_a = "a.txt"
        file_b = "b.txt"

        shared_text = "shared content between files"
        shared_hash = compute_hash(shared_text)

        _write_manifest_entry(ob_dir, shared_hash, file_a, ["s1"])
        _write_manifest_entry(ob_dir, shared_hash, file_b, ["s2"])
        _write_embedding(ob_dir, "test-model", shared_hash, [1.0, 0.0, 0.0, 0.0])

        new_text = "shared content between files MODIFIED"
        encode_fn = _make_encode_fn({new_text: [0.95, 0.05, 0.0, 0.0]})

        data_a = tmp_path / "a.txt"
        data_a.write_text(new_text + "\n")

        result = reconcile(
            str(data_a),
            model="test-model",
            threshold=0.85,
            ob_dir=ob_dir,
            _encode_fn=encode_fn,
        )

        assert result.semantic_matched == 1

        b_records = shard_read(ob_dir, "document-index", shared_hash)
        b_for_file = [r for r in b_records if r.get("file") == file_b]
        assert len(b_for_file) == 1
        assert b_for_file[0]["sources"] == ["s2"]


class TestArchiveRecords:
    def test_removed_record_is_archived(self, tmp_path):
        ob_dir = _init_ob_dir(tmp_path)
        file = "data.txt"

        old_text = "archived line"
        old_hash = compute_hash(old_text)
        _write_manifest_entry(ob_dir, old_hash, file, ["s1"])
        _write_embedding(ob_dir, "test-model", old_hash, [1.0, 0.0, 0.0, 0.0])

        new_text = "archived line modified"
        encode_fn = _make_encode_fn({new_text: [0.99, 0.01, 0.0, 0.0]})

        data_file = tmp_path / "data.txt"
        data_file.write_text(new_text + "\n")

        reconcile(
            str(data_file),
            model="test-model",
            threshold=0.85,
            ob_dir=ob_dir,
            _encode_fn=encode_fn,
        )

        bucket = old_hash[:2].lower()
        archive_path = ob_dir / ".ob" / "archive" / f"docidx.{bucket}"
        assert archive_path.exists()
        archived = jsonl_read(archive_path)
        assert any(
            r.get("line_hash") == old_hash and r.get("file") == file for r in archived
        )


class TestMakeApiEncodeFn:
    def test_returns_callable(self):
        from ob_util.reconcile import make_api_encode_fn

        fn = make_api_encode_fn("http://localhost:1234/v1", "test-model")
        assert callable(fn)

    def test_raises_on_http_error(self, tmp_path):
        from ob_util.reconcile import make_api_encode_fn
        from ob.exceptions import OBStorageError

        fn = make_api_encode_fn(
            "http://localhost:1/nonexistent", "test-model", timeout=1.0
        )
        with pytest.raises(OBStorageError, match="Embedding API request failed"):
            fn(["hello"])


# ---------------------------------------------------------------------------
# Error cases
# ---------------------------------------------------------------------------


class TestErrorCases:
    def test_missing_file_returns_error(self, tmp_path):
        ob_dir = _init_ob_dir(tmp_path)
        with pytest.raises(FileNotFoundError):
            reconcile(str(tmp_path / "nonexistent.txt"), ob_dir=ob_dir)
