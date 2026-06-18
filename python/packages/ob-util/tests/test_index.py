import json
from pathlib import Path

import pytest

from ob.storage import (
    LAYER_AUTHORS,
    LAYER_MANIFEST,
    LAYER_SECTION,
    jsonl_append,
    jsonl_read,
)
from ob_util.index import build_index


AUTHOR1 = "a" * 64
AUTHOR2 = "b" * 64
SECTION1 = "c" * 64
SECTION2 = "d" * 64
LINE1 = "e" * 64
LINE2 = "f" * 64


def _setup_ob_dir(tmp_path: Path) -> Path:
    ob_dot = tmp_path / ".ob"
    for layer in (LAYER_AUTHORS, LAYER_SECTION, LAYER_MANIFEST):
        (ob_dot / layer).mkdir(parents=True, exist_ok=True)

    for aid in (AUTHOR1, AUTHOR2):
        jsonl_append(
            ob_dot / LAYER_AUTHORS / aid[:2],
            {
                "id": aid,
                "name": f"Author_{aid[0]}",
                "email": f"{aid[0]}@test.com",
                "revoked": False,
            },
        )

    for sid in (SECTION1, SECTION2):
        jsonl_append(
            ob_dot / LAYER_SECTION / sid[:2],
            {
                "section_hash": sid,
                "path": "raw/wiki.xml",
                "authors": [AUTHOR1, AUTHOR2],
                "license": "CC-BY-SA-4.0",
                "year": "2024",
                "revoked": False,
            },
        )

    jsonl_append(
        ob_dot / LAYER_MANIFEST / LINE1[:2],
        {
            "line_hash": LINE1,
            "file": "data.jsonl",
            "sources": [SECTION1],
            "source_type": "track",
            "revoked": False,
        },
    )
    jsonl_append(
        ob_dot / LAYER_MANIFEST / LINE2[:2],
        {
            "line_hash": LINE2,
            "file": "data.jsonl",
            "sources": [SECTION2],
            "source_type": "track",
            "revoked": False,
        },
    )

    return tmp_path


def _read_index(ob_dir: Path) -> dict[str, dict]:
    index_dir = ob_dir / ".ob" / "index"
    result = {}
    if not index_dir.exists():
        return result
    for f in sorted(index_dir.iterdir()):
        if f.is_file():
            for rec in jsonl_read(f):
                result[rec["id"]] = rec
    return result


class TestBuildIndex:
    def test_build_index_creates_files(self, tmp_path):
        ob_dir = _setup_ob_dir(tmp_path)
        build_index(ob_dir)

        index_dir = ob_dir / ".ob" / "index"
        assert index_dir.exists()
        files = list(index_dir.iterdir())
        assert len(files) >= 2

    def test_build_index_author_refs(self, tmp_path):
        ob_dir = _setup_ob_dir(tmp_path)
        build_index(ob_dir)

        records = _read_index(ob_dir)
        assert AUTHOR1 in records
        assert set(records[AUTHOR1]["refs"]) == {SECTION1[:2], SECTION2[:2]}

    def test_build_index_section_refs(self, tmp_path):
        ob_dir = _setup_ob_dir(tmp_path)
        build_index(ob_dir)

        records = _read_index(ob_dir)
        assert SECTION1 in records
        assert records[SECTION1]["refs"] == [LINE1[:2]]

    def test_build_index_idempotent(self, tmp_path):
        ob_dir = _setup_ob_dir(tmp_path)
        first = build_index(ob_dir)
        second = build_index(ob_dir)

        assert first == second
        records = _read_index(ob_dir)
        for rec in records.values():
            assert "id" in rec and "refs" in rec

    def test_build_index_empty(self, tmp_path):
        ob_dir = tmp_path
        for layer in (LAYER_AUTHORS, LAYER_SECTION, LAYER_MANIFEST):
            (ob_dir / ".ob" / layer).mkdir(parents=True, exist_ok=True)

        counts = build_index(ob_dir)
        assert counts == {"authors": 0, "sections": 0, "total": 0}

        index_dir = ob_dir / ".ob" / "index"
        if index_dir.exists():
            assert len(list(index_dir.iterdir())) == 0

    def test_build_index_format(self, tmp_path):
        ob_dir = _setup_ob_dir(tmp_path)
        build_index(ob_dir)

        index_dir = ob_dir / ".ob" / "index"
        for f in index_dir.iterdir():
            if not f.is_file():
                continue
            for line in f.read_text().strip().split("\n"):
                rec = json.loads(line)
                assert isinstance(rec["id"], str)
                assert isinstance(rec["refs"], list)
                assert all(isinstance(r, str) and len(r) == 2 for r in rec["refs"])
