# -*- coding: utf-8 -*-
"""Regression tests for the stat-guarded author-id resolver in ob.api.

Covers the 0.2.2 fix: resolving names must not re-scan the whole authors
layer per call, must pick up externally appended authors (signature
invalidation), and must raise instead of silently dropping unknown names.
Runs without the native extension (shards are written by hand).
"""
import json
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from ob.api import _resolve_author_ids  # noqa: E402
from ob.exceptions import OBSectionError  # noqa: E402


def _write_shard(root: Path, records: list[dict]) -> None:
    """按真实账本布局落盘:每条记录写进 id 前两位对应的桶。"""
    by_bucket: dict[str, list[dict]] = {}
    for r in records:
        by_bucket.setdefault(r["id"][:2], []).append(r)
    authors_dir = root / ".ob" / "authors"
    authors_dir.mkdir(parents=True, exist_ok=True)
    for bucket, recs in by_bucket.items():
        with (authors_dir / bucket).open("a", encoding="utf-8") as f:
            for r in recs:
                f.write(json.dumps(r, ensure_ascii=False) + "\n")


def _rec(aid: str, name: str) -> dict:
    return {"id": aid, "name": name, "email": f"{name}@x.invalid", "revoked": False}


def _fresh(tmp: Path) -> Path:
    root = tmp / "repo"
    (root / ".ob" / "authors").mkdir(parents=True, exist_ok=True)
    _resolve_author_ids.__globals__["_author_state"].clear()
    return root


def test_resolves_known_names(tmp_path: Path) -> None:
    root = _fresh(tmp_path)
    _write_shard(root, [_rec("a" * 64, "张三"), _rec("b" * 64, "李四")])
    assert _resolve_author_ids(["张三", "李四"], root) == ["a" * 64, "b" * 64]


def test_unknown_name_raises(tmp_path: Path) -> None:
    root = _fresh(tmp_path)
    _write_shard(root, [_rec("a" * 64, "张三")])
    try:
        _resolve_author_ids(["张三", "不存在"], root)
    except OBSectionError as e:
        assert "不存在" in str(e)
    else:
        raise AssertionError("expected OBSectionError for unresolvable name")


def test_external_append_is_seen_without_restart(tmp_path: Path) -> None:
    root = _fresh(tmp_path)
    _write_shard(root, [_rec("a" * 64, "张三")])
    assert _resolve_author_ids(["张三"], root) == ["a" * 64]

    _write_shard(root, [_rec("c" * 64, "王五")])  # 模拟另一进程(native)追加
    _resolve_author_ids(["王五"], root)  # 不得抛错:签名失效须触发重扫


def test_ambiguous_name_is_deterministic(tmp_path: Path) -> None:
    root = _fresh(tmp_path)
    _write_shard(root, [_rec("f" * 64, "同名人"), _rec("0" * 64, "同名人")])
    first = _resolve_author_ids(["同名人"], root)
    again = _resolve_author_ids(["同名人"], root)
    assert first == again == [sorted(first)[0]]


def test_repeated_resolves_do_not_rescan(tmp_path: Path) -> None:
    root = _fresh(tmp_path)
    records = [_rec(f"{i:064x}", f"作者{i}") for i in range(2000)]
    _write_shard(root, records)
    _resolve_author_ids([records[0]["name"]], root)  # 首次:全量扫描+灌缓存

    # chmod 不改 size/mtime → 签名不变 → 缓存路径不得读盘;回退成每次重扫则必炸
    for f in (root / ".ob" / "authors").iterdir():
        f.chmod(0o000)
    names = [r["name"] for r in records]
    t0 = time.perf_counter()
    for _ in range(50):
        _resolve_author_ids(names, root)
    dt = time.perf_counter() - t0
    assert dt < 10.0, f"cached resolves too slow: {dt:.2f}s"


def test_shard_isolation_incremental_refresh(tmp_path: Path) -> None:
    root = _fresh(tmp_path)
    a_recs = [_rec(f"{i:064x}", f"桶a作者{i}") for i in range(500)]     # id 前缀 00~01
    b_recs = [_rec(f"{i + 0xa00:064x}", f"桶b作者{i}") for i in range(500)]  # id 前缀 0a
    _write_shard(root, a_recs + b_recs)
    _resolve_author_ids([a_recs[0]["name"]], root)  # 预热全部分片

    # 冻结桶 00,向其他桶追加:增量刷新不得读 00,且桶 00 旧名仍可解析
    (root / ".ob" / "authors" / "00").chmod(0o000)
    _write_shard(root, [_rec("b" * 64, "桶b新作者")])
    ids = _resolve_author_ids(["桶b新作者", a_recs[0]["name"], a_recs[499]["name"]], root)
    assert ids == ["b" * 64, a_recs[0]["id"], a_recs[499]["id"]]


if __name__ == "__main__":
    import tempfile

    failures = 0
    for fn in [
        test_resolves_known_names,
        test_unknown_name_raises,
        test_external_append_is_seen_without_restart,
        test_ambiguous_name_is_deterministic,
        test_repeated_resolves_do_not_rescan,
        test_shard_isolation_incremental_refresh,
    ]:
        with tempfile.TemporaryDirectory() as d:
            try:
                fn(Path(d))
                print(f"PASS {fn.__name__}")
            except Exception as e:  # noqa: BLE001
                failures += 1
                print(f"FAIL {fn.__name__}: {e!r}")
    sys.exit(1 if failures else 0)
