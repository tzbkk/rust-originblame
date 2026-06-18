from pathlib import Path

import pytest

from ob.api import init as ob_init
from ob.authors import add_author
from ob.exceptions import OBStorageError
from ob.register import register_section
from ob.storage import shard_append, jsonl_append

from ob_util.export import export_copyright

LINE_A = "a0" + "0" * 62
LINE_B = "b0" + "0" * 62
LINE_C = "c0" + "0" * 62
LINE_D = "d0" + "0" * 62
LINE_E = "e0" + "0" * 62
LINE_F = "f0" + "0" * 62
LINE_A2 = "a1" + "0" * 62


def _setup_ob_dir(tmp_path: Path) -> Path:
    ob_init(ob_dir=tmp_path, force=True)
    return tmp_path


def _add_author(ob_dir: Path, name: str, email: str) -> str:
    return add_author(ob_dir, name, email)


def _add_section(ob_dir: Path, path: str, author_ids: list[str], license: str, year: str) -> str:
    return register_section(ob_dir, path, author_ids, license, year)


def _add_index_record(ob_dir: Path, line_hash: str, file: str, sources: list[str]) -> None:
    shard_append(ob_dir, "document-index", line_hash, {
        "line_hash": line_hash,
        "file": file,
        "sources": sources,
    })


def _create_pid_file(ob_dir: Path, pid: int) -> None:
    jsonl_append(ob_dir / ".ob" / f"docidx.{pid}", {"test": True})


class TestExportFormat:
    def test_produces_dep5_format(self, tmp_path):
        ob_dir = _setup_ob_dir(tmp_path)
        aid = _add_author(ob_dir, "Alice", "alice@example.com")
        sh = _add_section(ob_dir, "raw/wiki.xml", [aid], "CC-BY-SA-4.0", "2024")
        _add_index_record(ob_dir, LINE_A, "data/train.jsonl", [sh])

        result = export_copyright(ob_dir=ob_dir)

        assert f"Files: data/train.jsonl:{LINE_A}" in result
        assert "Copyright: 2024 Alice <alice@example.com>" in result
        assert "License: CC-BY-SA-4.0" in result

    def test_multiple_authors_in_copyright(self, tmp_path):
        ob_dir = _setup_ob_dir(tmp_path)
        aid1 = _add_author(ob_dir, "Alice", "alice@example.com")
        aid2 = _add_author(ob_dir, "Bob", "bob@example.com")
        sh = _add_section(ob_dir, "raw/wiki.xml", [aid1, aid2], "CC-BY-SA-4.0", "2024")
        _add_index_record(ob_dir, LINE_B, "data/train.jsonl", [sh])

        result = export_copyright(ob_dir=ob_dir)

        assert "Copyright: 2024 Alice <alice@example.com>" in result
        assert "Copyright: 2024 Bob <bob@example.com>" in result

    def test_multiple_sources_generates_multiple_blocks(self, tmp_path):
        ob_dir = _setup_ob_dir(tmp_path)
        aid1 = _add_author(ob_dir, "Alice", "alice@example.com")
        aid2 = _add_author(ob_dir, "Bob", "bob@example.com")
        sh1 = _add_section(ob_dir, "raw/wiki.xml", [aid1], "CC-BY-SA-4.0", "2024")
        sh2 = _add_section(ob_dir, "raw/gpt.xml", [aid2], "MIT", "2023")
        _add_index_record(ob_dir, LINE_C, "data/train.jsonl", [sh1, sh2])

        result = export_copyright(ob_dir=ob_dir)

        assert "License: CC-BY-SA-4.0" in result
        assert "License: MIT" in result
        assert result.count(f"Files: data/train.jsonl:{LINE_C}") == 2


class TestExportFilterByDataFile:
    def test_filters_by_single_data_file(self, tmp_path):
        ob_dir = _setup_ob_dir(tmp_path)
        aid = _add_author(ob_dir, "Alice", "alice@example.com")
        sh = _add_section(ob_dir, "raw/wiki.xml", [aid], "CC-BY-SA-4.0", "2024")
        _add_index_record(ob_dir, LINE_A, "data/train.jsonl", [sh])
        _add_index_record(ob_dir, LINE_B, "data/test.jsonl", [sh])

        result = export_copyright(ob_dir=ob_dir, data_files=["data/train.jsonl"])

        assert LINE_A in result
        assert LINE_B not in result

    def test_filters_by_multiple_data_files(self, tmp_path):
        ob_dir = _setup_ob_dir(tmp_path)
        aid = _add_author(ob_dir, "Alice", "alice@example.com")
        sh = _add_section(ob_dir, "raw/wiki.xml", [aid], "CC-BY-SA-4.0", "2024")
        _add_index_record(ob_dir, LINE_A, "data/train.jsonl", [sh])
        _add_index_record(ob_dir, LINE_B, "data/test.jsonl", [sh])
        _add_index_record(ob_dir, LINE_C, "data/other.jsonl", [sh])

        result = export_copyright(
            ob_dir=ob_dir,
            data_files=["data/train.jsonl", "data/test.jsonl"],
        )

        assert LINE_A in result
        assert LINE_B in result
        assert LINE_C not in result


class TestExportToFile:
    def test_writes_to_file(self, tmp_path):
        ob_dir = _setup_ob_dir(tmp_path)
        aid = _add_author(ob_dir, "Alice", "alice@example.com")
        sh = _add_section(ob_dir, "raw/wiki.xml", [aid], "CC-BY-SA-4.0", "2024")
        _add_index_record(ob_dir, LINE_A, "data/train.jsonl", [sh])

        output_file = tmp_path / "debian" / "copyright"
        result = export_copyright(ob_dir=ob_dir, output=str(output_file))

        assert output_file.exists()
        assert output_file.read_text(encoding="utf-8") == result

    def test_creates_parent_directories(self, tmp_path):
        ob_dir = _setup_ob_dir(tmp_path)
        aid = _add_author(ob_dir, "Alice", "alice@example.com")
        sh = _add_section(ob_dir, "raw/wiki.xml", [aid], "CC-BY-SA-4.0", "2024")
        _add_index_record(ob_dir, LINE_A, "data/train.jsonl", [sh])

        output_file = tmp_path / "a" / "b" / "c" / "copyright"
        export_copyright(ob_dir=ob_dir, output=str(output_file))

        assert output_file.exists()


class TestExportUnknownLicense:
    def test_empty_license_outputs_unknown(self, tmp_path):
        ob_dir = _setup_ob_dir(tmp_path)
        aid = _add_author(ob_dir, "Bob", "bob@example.com")
        sh = _add_section(ob_dir, "raw/unknown.xml", [aid], "", "2024")
        _add_index_record(ob_dir, LINE_D, "data/train.jsonl", [sh])

        result = export_copyright(ob_dir=ob_dir)

        assert "License: UNKNOWN" in result
        assert "CC-BY-SA" not in result

    def test_missing_license_key_outputs_unknown(self, tmp_path):
        ob_dir = _setup_ob_dir(tmp_path)
        aid = _add_author(ob_dir, "Bob", "bob@example.com")
        sh = _add_section(ob_dir, "raw/unknown.xml", [aid], "", "2024")
        _add_index_record(ob_dir, LINE_E, "data/train.jsonl", [sh])

        shard_append(ob_dir, "sections", sh, {
            "section_hash": sh,
            "path": "raw/nolicense.xml",
            "authors": [aid],
            "year": "2024",
            "revoked": False,
        })
        _add_index_record(ob_dir, LINE_F, "data/train.jsonl", [sh])

        result = export_copyright(ob_dir=ob_dir)

        assert "License: UNKNOWN" in result


class TestExportNoRecords:
    def test_no_manifest_records_returns_empty_string(self, tmp_path):
        ob_dir = _setup_ob_dir(tmp_path)

        result = export_copyright(ob_dir=ob_dir)

        assert result == ""

    def test_filter_excludes_all_records(self, tmp_path):
        ob_dir = _setup_ob_dir(tmp_path)
        aid = _add_author(ob_dir, "Alice", "alice@example.com")
        sh = _add_section(ob_dir, "raw/wiki.xml", [aid], "CC-BY-SA-4.0", "2024")
        _add_index_record(ob_dir, LINE_A, "data/train.jsonl", [sh])

        result = export_copyright(ob_dir=ob_dir, data_files=["nonexistent.jsonl"])

        assert result == ""


class TestExportUnmergedPidFiles:
    def test_raises_on_unmerged_pid_files(self, tmp_path):
        ob_dir = _setup_ob_dir(tmp_path)
        _create_pid_file(ob_dir, 12345)

        with pytest.raises(OBStorageError, match="Unmerged PID files"):
            export_copyright(ob_dir=ob_dir)
