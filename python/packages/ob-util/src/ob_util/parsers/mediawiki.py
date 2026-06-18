"""MediaWiki XML dump parser for OriginBlame.

Independent preprocessing step -- does NOT call source.append or track.

Supports per-line blame attribution via forward-annotate diff algorithm
and section splitting by ``== Heading ==`` boundaries.
"""

from __future__ import annotations

import base64
import difflib
import hashlib
import logging
import re
import warnings
from dataclasses import dataclass, field
from pathlib import Path

try:
    from _ob_native import blame_diff as _native_blame_diff
except ImportError:
    _native_blame_diff = None  # type: ignore[assignment]


def _diff_opcodes(old: list[str], new: list[str]) -> list[tuple[str, int, int, int, int]]:
    if _native_blame_diff is not None:
        return list(_native_blame_diff(old, new))  # type: ignore[arg-type]
    return difflib.SequenceMatcher(None, old, new, autojunk=False).get_opcodes()

logger = logging.getLogger(__name__)

_MAX_FILENAME = 200
_MAX_PAGE_BYTES = 50 * 1024 * 1024
_MIN_CHUNK_CHARS = 400
_MIN_CLEAN_CHARS = 100

_RE_TITLE = re.compile(r"<title>(.*?)</title>", re.DOTALL)
_RE_NS = re.compile(r"<ns>(.*?)</ns>")
_RE_USERNAME = re.compile(r"<username>(.*?)</username>")
_RE_IP = re.compile(r"<ip>(.*?)</ip>")
_RE_SHA1 = re.compile(r"<sha1>(.*?)</sha1>")
_RE_TIMESTAMP = re.compile(r"<timestamp>(.*?)</timestamp>")
_RE_TEXT_START = re.compile(r"<text[^>]*>")
_RE_TEXT_END = re.compile(r"</text>")

_RE_HEADING = re.compile(r"^== (?!=)(.+?)(?<!=) ==$", re.MULTILINE)

_BOILERPLATE_KEYWORDS = frozenset(
    ["参", "注", "来源", "外部", "延伸", "扩展", "书目", "导航", "参考"]
)


# ---------------------------------------------------------------------------
# Data classes
# ---------------------------------------------------------------------------


@dataclass
class ParseResult:
    pages_parsed: int = 0
    authors_registered: int = 0
    sections_created: int = 0
    split_files_created: int = 0


@dataclass
class ContentChunk:
    """A section-level chunk of wiki content with provenance metadata."""

    page_title: str
    heading: str  # "历史" / "地理+经济" (merged) / "[INTRO]"
    text: str  # cleaned wikitext (markup stripped)
    raw_text: str  # original wikitext (for tokenization)
    authors: list[str] = field(default_factory=list)  # author IDs
    year: str = ""  # latest revision year for lines in this chunk
    start_line: int = 0  # 0-based line index in final text
    end_line: int = 0  # exclusive
    source_path: str = ""  # "raw/北京#历史"


# ---------------------------------------------------------------------------
# Helper: markup stripping (minimal, for clean-text estimation)
# ---------------------------------------------------------------------------

_RE_MARKUP = re.compile(
    r"""
    \'{2,5}        # bold/italic markup
    | \[\[         # wiki link open
    | \]\]         # wiki link close
    | \{\{.*?\}\}  # template call (single-line)
    | \|--\}       # table end
    | \{\|         # table start
    | \|-          # table row
    | \|           # table cell (loose)
    | <ref.*?</ref>  # ref tags (single-line)
    | <[^>]+/>     # self-closing html tags
    | <[^>]+>      # opening html tags
    | </[^>]+>     # closing html tags
    | ={2,}        # heading markup
    | \*+          # bullet list
    | \#+          # numbered list
    | ;+           # definition list
    | :+           # indent
    """,
    re.VERBOSE,
)

def _strip_markup(text: str) -> str:
    """Strip wiki markup for a clean-text estimate."""
    text = _RE_MARKUP.sub("", text)
    # Remove remaining [[ ... ]] content markers
    text = re.sub(r"[\[\]]{2}", "", text)
    return text.strip()


# ---------------------------------------------------------------------------
# Section splitting
# ---------------------------------------------------------------------------


def split_sections(
    wikitext: str,
    page_title: str = "",
    line_attributions: list[tuple[str, str]] | None = None,
) -> list[ContentChunk]:
    """Split wikitext by ``== Heading ==`` boundaries.

    Args:
        wikitext: The full wikitext of the final revision.
        page_title: Page title (used in source_path).
        line_attributions: Optional per-line ``(text, author_id)`` pairs
            from :func:`blame_revisions`.  If provided, chunk authors are
            derived from the attributed lines within each chunk.

    Returns:
        List of :class:`ContentChunk` instances.
    """
    if not wikitext.strip():
        return []

    # Split by == Heading == boundaries
    matches = list(_RE_HEADING.finditer(wikitext))

    raw_chunks: list[tuple[str, str]] = []  # (heading, raw_text)

    if not matches:
        # No headings -- entire page is one chunk
        raw_chunks.append(("[INTRO]", wikitext))
    else:
        # Intro section (before first heading)
        intro = wikitext[: matches[0].start()].strip()
        if intro:
            raw_chunks.append(("[INTRO]", intro))

        for i, m in enumerate(matches):
            heading = m.group(1).strip()
            start = m.end()
            end = matches[i + 1].start() if i + 1 < len(matches) else len(wikitext)
            section_text = wikitext[start:end].strip()
            raw_chunks.append((heading, section_text))

    # Filter boilerplate sections
    filtered: list[tuple[str, str]] = []
    for heading, text in raw_chunks:
        if any(kw in heading for kw in _BOILERPLATE_KEYWORDS):
            continue
        filtered.append((heading, text))

    if not filtered:
        return []

    # Merge small chunks (< _MIN_CHUNK_CHARS)
    merged: list[tuple[str, str]] = []
    for heading, text in filtered:
        if merged and len(text) < _MIN_CHUNK_CHARS:
            # Merge into previous chunk
            prev_h, prev_t = merged[-1]
            merged[-1] = (
                prev_h + "+" + heading,
                prev_t + "\n" + text,
            )
        elif len(text) < _MIN_CHUNK_CHARS and len(filtered) > 1:
            # Small chunk but nothing to merge with yet -- stash it
            merged.append((heading, text))
        else:
            merged.append((heading, text))

    # Final merge pass: if last chunk is still small, merge into previous
    if len(merged) >= 2 and len(merged[-1][1]) < _MIN_CHUNK_CHARS:
        last_h, last_t = merged.pop()
        prev_h, prev_t = merged[-1]
        merged[-1] = (
            prev_h + "+" + last_h,
            prev_t + "\n" + last_t,
        )

    # Build ContentChunk instances
    chunks: list[ContentChunk] = []
    line_offset = 0

    for heading, raw_text in merged:
        clean_text = _strip_markup(raw_text)
        lines_count = raw_text.count("\n") + 1
        end_line = line_offset + lines_count

        # Determine authors from line attributions
        authors: list[str] = []
        year = ""
        if line_attributions:
            chunk_authors: set[str] = set()
            # Collect authors for lines in this chunk's range
            for li, (line_text, author_id) in enumerate(line_attributions):
                if line_offset <= li < end_line:
                    if author_id:
                        chunk_authors.add(author_id)
            authors = sorted(chunk_authors)

        source_path = f"raw/{page_title}#{heading}" if page_title else ""

        chunks.append(
            ContentChunk(
                page_title=page_title,
                heading=heading,
                text=clean_text,
                raw_text=raw_text,
                authors=authors,
                year=year,
                start_line=line_offset,
                end_line=end_line,
                source_path=source_path,
            )
        )
        line_offset = end_line

    return chunks


# ---------------------------------------------------------------------------
# Forward-annotate blame
# ---------------------------------------------------------------------------


def blame_revisions(
    revisions: list[dict],
) -> list[tuple[str, str]]:
    """Forward-annotate diff: attribute each line of the FINAL text to an author.

    Args:
        revisions: Sorted list of revision dicts, each with keys:
            ``timestamp``, ``contributor``, ``text``, ``sha1`` (optional).

    Returns:
        List of ``(line_text, author_id)`` tuples, one per line of the final
        revision's text.  ``author_id`` is the contributor name/IP responsible
        for that line (last-modifier-wins, same as git blame).
    """
    if not revisions:
        return []

    # Sort by timestamp
    sorted_revs = sorted(revisions, key=lambda r: r.get("timestamp", ""))

    # Start with empty attribution
    current_lines: list[tuple[str, str]] = []  # (line_text, author_id)

    prev_sha1 = None

    for rev in sorted_revs:
        sha1 = rev.get("sha1")
        author = rev.get("contributor", "")
        text = rev.get("text", "")

        # Skip if sha1 unchanged (memory optimization)
        if sha1 and sha1 == prev_sha1:
            continue
        prev_sha1 = sha1

        new_lines = text.splitlines(True) if text else []

        # First revision: everything attributed to this author
        if not current_lines:
            current_lines = [(line, author) for line in new_lines]
            continue

        old_texts = [line for line, _ in current_lines]
        result: list[tuple[str, str]] = []
        for tag, i1, i2, j1, j2 in _diff_opcodes(old_texts, new_lines):
            if tag == "equal":
                # Keep existing attribution
                for k in range(i1, i2):
                    result.append(current_lines[k])
            elif tag == "replace":
                # New lines get new author
                for k in range(j1, j2):
                    result.append((new_lines[k], author))
            elif tag == "insert":
                # New lines get new author
                for k in range(j1, j2):
                    result.append((new_lines[k], author))
            elif tag == "delete":
                # Deleted lines are simply dropped
                pass

        current_lines = result

    return current_lines


# ---------------------------------------------------------------------------
# XML stream parsing helpers
# ---------------------------------------------------------------------------


def _safe_filename(title: str) -> str:
    """Base64url encode title, truncate to _MAX_FILENAME."""
    raw = base64.urlsafe_b64encode(title.encode("utf-8")).decode("ascii")
    if len(raw) <= _MAX_FILENAME:
        return raw
    h = hashlib.sha256(title.encode("utf-8")).hexdigest()[:8]
    return raw[: _MAX_FILENAME - 9] + "_" + h


def _extract_page_info(lines: list[str]) -> dict | None:
    """Stream-parse page lines for title, ns, contributors, latest timestamp, latest text.

    This is the backward-compatible single-revision interface.  For multi-revision
    parsing, use :func:`_extract_all_revisions` instead.
    """
    title = None
    ns_val = None
    usernames: set[str] = set()
    ips: set[str] = set()
    last_timestamp = None
    latest_text_lines: list[str] = []
    collecting_text = False
    total_bytes = 0

    for line in lines:
        total_bytes += len(line)
        if total_bytes > _MAX_PAGE_BYTES:
            return None

        if title is None:
            m = _RE_TITLE.search(line)
            if m:
                title = m.group(1).strip()
                continue

        if ns_val is None:
            m = _RE_NS.search(line)
            if m:
                ns_val = m.group(1).strip()
                if ns_val != "0":
                    return None
                continue

        for n in _RE_USERNAME.findall(line):
            usernames.add(n.strip())
        for ip in _RE_IP.findall(line):
            ips.add(ip.strip())

        m = _RE_TIMESTAMP.search(line)
        if m:
            last_timestamp = m.group(1).strip()[:4]
            latest_text_lines = []
            collecting_text = False

        if collecting_text:
            if _RE_TEXT_END.search(line):
                collecting_text = False
            else:
                latest_text_lines.append(line)
        elif _RE_TEXT_START.search(line):
            collecting_text = True
            tag_end = line.find(">")
            if tag_end != -1:
                remainder = line[tag_end + 1 :]
                if _RE_TEXT_END.search(remainder):
                    collecting_text = False
                else:
                    latest_text_lines.append(remainder)

    if not title or last_timestamp is None:
        return None

    wikitext = "".join(latest_text_lines).strip()

    contributors: list[str] = []
    seen: set[str] = set()
    for name in sorted(usernames):
        if name and name not in seen:
            seen.add(name)
            contributors.append(name)
    for ip in sorted(ips):
        if ip and ip not in seen:
            seen.add(ip)
            contributors.append(ip)

    return {
        "title": title,
        "year": last_timestamp,
        "wikitext": wikitext,
        "contributors": contributors,
    }


def _extract_all_revisions(lines: list[str]) -> dict | None:
    """Stream-parse page lines with streaming blame and bot merging.

    Computes per-line blame incrementally during XML parsing instead of
    collecting all revisions first.  Uses bot merging: consecutive revisions
    by the same author skip the expensive SequenceMatcher diff (O(L) instead
    of O(L²)).

    Returns a dict with:
        - title, year (latest), contributors (all unique)
        - revisions: list of {timestamp, contributor, text, sha1}
        - latest_wikitext: the text from the last revision
        - chunks: list[ContentChunk] (if section splitting yields results)
    """
    title = None
    ns_val = None
    total_bytes = 0

    # Revision metadata for return value (kept for backward compat)
    revisions: list[dict] = []
    usernames: set[str] = set()
    ips: set[str] = set()

    # Current revision parsing state
    current_timestamp: str | None = None
    current_contributor: str | None = None
    saved_contributor: str | None = None
    current_sha1: str | None = None
    current_text_lines: list[str] = []
    collecting_text = False
    rev_count = 0

    # Streaming blame state — only holds current attribution + prev text
    blame_lines: list[tuple[str, str]] = []  # (line_text, author_id)
    prev_text: str | None = None
    prev_sha1: str | None = None
    prev_author: str | None = None

    for line in lines:
        total_bytes += len(line)
        if total_bytes > _MAX_PAGE_BYTES:
            return None

        if title is None:
            m = _RE_TITLE.search(line)
            if m:
                title = m.group(1).strip()
                continue

        if ns_val is None:
            m = _RE_NS.search(line)
            if m:
                ns_val = m.group(1).strip()
                if ns_val != "0":
                    return None
                continue

        for n in _RE_USERNAME.findall(line):
            current_contributor = n.strip()
            usernames.add(n.strip())
        for ip in _RE_IP.findall(line):
            current_contributor = ip.strip()
            ips.add(ip.strip())

        m_sha = _RE_SHA1.search(line)
        if m_sha:
            current_sha1 = m_sha.group(1).strip()

        m = _RE_TIMESTAMP.search(line)
        if m:
            # Flush previous revision (streaming blame + metadata)
            if current_timestamp is not None:
                rev_text = "".join(current_text_lines).strip()
                contributor = saved_contributor or ""
                sha1 = current_sha1

                revisions.append(
                    {
                        "timestamp": current_timestamp,
                        "contributor": contributor,
                        "text": rev_text,
                        "sha1": sha1,
                    }
                )
                rev_count += 1

                # --- Streaming blame for this revision ---
                # SHA-1 dedup (adjacent identical content)
                if sha1 and sha1 == prev_sha1:
                    pass  # skip — attribution unchanged
                else:
                    new_lines = rev_text.splitlines(True) if rev_text else []
                    if prev_text is None:
                        # First revision: attribute all lines to this author
                        blame_lines = [(l, contributor) for l in new_lines]
                    elif contributor == prev_author:
                        # Bot merge: same author as previous revision.
                        # All lines in the new text would be attributed to this
                        # author regardless, so skip SequenceMatcher (O(L) vs O(L²)).
                        blame_lines = [(l, contributor) for l in new_lines]
                    else:
                        old_texts = [l for l, _ in blame_lines]
                        result: list[tuple[str, str]] = []
                        for tag, i1, i2, j1, j2 in _diff_opcodes(old_texts, new_lines):
                            if tag == "equal":
                                for k in range(i1, i2):
                                    result.append(blame_lines[k])
                            elif tag in ("replace", "insert"):
                                for k in range(j1, j2):
                                    result.append((new_lines[k], contributor))
                            # "delete": lines dropped
                        blame_lines = result

                    prev_text = rev_text
                    prev_sha1 = sha1
                    prev_author = contributor

            # Start new revision
            current_timestamp = m.group(1).strip()[:4]
            saved_contributor = current_contributor
            current_text_lines = []
            current_sha1 = None
            collecting_text = False

        # Text collection
        if collecting_text:
            if _RE_TEXT_END.search(line):
                collecting_text = False
            else:
                current_text_lines.append(line)
        elif _RE_TEXT_START.search(line):
            collecting_text = True
            tag_end = line.find(">")
            if tag_end != -1:
                remainder = line[tag_end + 1 :]
                if _RE_TEXT_END.search(remainder):
                    collecting_text = False
                else:
                    current_text_lines.append(remainder)

    # Flush last revision
    if current_timestamp is not None:
        rev_text = "".join(current_text_lines).strip()
        contributor = current_contributor or ""
        sha1 = current_sha1

        revisions.append(
            {
                "timestamp": current_timestamp,
                "contributor": contributor,
                "text": rev_text,
                "sha1": sha1,
            }
        )
        rev_count += 1

        # Streaming blame for last revision
        if sha1 and sha1 == prev_sha1:
            pass
        else:
            new_lines = rev_text.splitlines(True) if rev_text else []
            if prev_text is None:
                blame_lines = [(l, contributor) for l in new_lines]
            elif contributor == prev_author:
                # Bot merge
                blame_lines = [(l, contributor) for l in new_lines]
            else:
                old_texts = [l for l, _ in blame_lines]
                result: list[tuple[str, str]] = []
                for tag, i1, i2, j1, j2 in _diff_opcodes(old_texts, new_lines):
                    if tag == "equal":
                        for k in range(i1, i2):
                            result.append(blame_lines[k])
                    elif tag in ("replace", "insert"):
                        for k in range(j1, j2):
                            result.append((new_lines[k], contributor))
                blame_lines = result

            prev_text = rev_text
            prev_sha1 = sha1
            prev_author = contributor

    if not title or not revisions:
        return None

    latest_wikitext = revisions[-1]["text"] if revisions else ""
    latest_year = revisions[-1]["timestamp"] if revisions else ""

    contributors: list[str] = []
    seen: set[str] = set()
    for name in sorted(usernames):
        if name and name not in seen:
            seen.add(name)
            contributors.append(name)
    for ip in sorted(ips):
        if ip and ip not in seen:
            seen.add(ip)
            contributors.append(ip)

    # Use streaming blame results directly (no separate blame_revisions call)
    line_attrs = blame_lines

    # Split into sections
    chunks = split_sections(latest_wikitext, page_title=title, line_attributions=line_attrs)

    # If no chunks (e.g. empty text), fall back to single-chunk
    if not chunks and latest_wikitext:
        chunks = [
            ContentChunk(
                page_title=title,
                heading="[INTRO]",
                text=_strip_markup(latest_wikitext),
                raw_text=latest_wikitext,
                authors=[a for _, a in line_attrs if a],
                year=latest_year,
                start_line=0,
                end_line=len(latest_wikitext.splitlines()),
                source_path=f"raw/{title}#[INTRO]",
            )
        ]

    return {
        "title": title,
        "year": latest_year,
        "wikitext": latest_wikitext,
        "contributors": contributors,
        "revisions": revisions,
        "chunks": chunks,
        "line_attributions": line_attrs,
    }


# ---------------------------------------------------------------------------
# Parser class
# ---------------------------------------------------------------------------


class MediawikiParser:
    """Parse MediaWiki XML dumps and register authors/sections in .ob/."""

    def __init__(self, ob_dir: Path | None = None, license: str = "CC-BY-SA-4.0"):
        if ob_dir is None:
            from ob.util import find_ob_dir

            ob_dir = find_ob_dir()
        self.ob_dir = ob_dir
        self.license = license

    def parse(self, file: str, **kwargs) -> ParseResult:
        do_split = kwargs.get("split", False)
        split_only = kwargs.get("split_only", False)
        blame_mode = kwargs.get("blame", False)
        result = ParseResult()
        seen_authors: set[str] = set()

        buf: list[str] = []
        in_page = False
        page_bytes = 0

        with open(file, encoding="utf-8") as f:
            for line in f:
                if "<page>" in line:
                    in_page = True
                    buf = [line]
                    page_bytes = len(line)
                    continue
                if in_page:
                    page_bytes += len(line)
                    if page_bytes > _MAX_PAGE_BYTES:
                        if "</page>" in line:
                            result.pages_parsed += 1
                            in_page = False
                            buf = []
                        continue
                    buf.append(line)
                    if "</page>" in line:
                        in_page = False
                        if blame_mode:
                            self._parse_page_blame(
                                buf, do_split, split_only, result, seen_authors
                            )
                        else:
                            self._parse_page_chunk(
                                buf, do_split, split_only, result, seen_authors
                            )
                        buf = []

        return result

    def _parse_page_blame(
        self,
        lines: list[str],
        do_split: bool,
        split_only: bool,
        result: ParseResult,
        seen_authors: set[str],
    ) -> None:
        """Parse a page with full revision history and per-chunk attribution."""
        info = _extract_all_revisions(lines)
        result.pages_parsed += 1
        if info is None:
            return

        title = info["title"]
        contributors = info["contributors"]
        chunks = info.get("chunks", [])

        if split_only:
            if do_split:
                self._write_split_file(title, info["year"], info["wikitext"], result)
            return

        # Register all contributors as authors
        author_ids = self._register_contributors(contributors, result, seen_authors)
        if not author_ids:
            return

        from ob.api import register_section

        # Build author name -> author_id lookup
        name_to_id: dict[str, str] = {}
        for name in contributors:
            for aid in author_ids:
                # author_add uses name as part of the ID computation
                # We need to map back from name to the registered ID
                pass

        # If no chunks, register as single section (backward compat)
        if not chunks:
            register_section(
                path=f"raw/{title}",
                authors=author_ids,
                license=self.license,
                year=info["year"],
                ob_dir=self.ob_dir,
            )
            result.sections_created += 1
            return

        # Register each chunk as a separate section
        for chunk in chunks:
            # Resolve chunk author names to IDs
            chunk_author_ids = self._resolve_chunk_authors(
                chunk.authors, author_ids, contributors
            )
            if not chunk_author_ids:
                chunk_author_ids = author_ids  # fallback to all authors

            register_section(
                path=chunk.source_path,
                authors=chunk_author_ids,
                license=self.license,
                year=info["year"],
                ob_dir=self.ob_dir,
            )
            result.sections_created += 1

        if do_split:
            self._write_split_file(title, info["year"], info["wikitext"], result)

    def _resolve_chunk_authors(
        self,
        chunk_authors: list[str],
        all_author_ids: list[str],
        contributor_names: list[str],
    ) -> list[str]:
        """Resolve chunk-level author names/IDs to registered author IDs."""
        if not chunk_authors:
            return []

        # chunk_authors are already contributor names from blame_revisions
        # Map them to author IDs via re-registration lookup
        from ob.api import author_add

        resolved: list[str] = []
        seen: set[str] = set()
        for name in chunk_authors:
            if name in seen:
                continue
            seen.add(name)
            # Register (idempotent) and get the ID
            aid = author_add(
                name=name, email=f"{name}@mediawiki", ob_dir=self.ob_dir
            )
            if aid not in resolved:
                resolved.append(aid)
        return resolved

    def _parse_page_chunk(
        self,
        lines: list[str],
        do_split: bool,
        split_only: bool,
        result: ParseResult,
        seen_authors: set[str],
    ) -> None:
        info = _extract_page_info(lines)
        result.pages_parsed += 1
        if info is None:
            return

        title = info["title"]
        contributors = info["contributors"]

        if split_only:
            if do_split:
                self._write_split_file(title, info["year"], info["wikitext"], result)
            return

        author_ids = self._register_contributors(contributors, result, seen_authors)
        if not author_ids:
            return

        from ob.api import register_section

        register_section(
            path=f"raw/{title}",
            authors=author_ids,
            license=self.license,
            year=info["year"],
            ob_dir=self.ob_dir,
        )
        result.sections_created += 1

        if do_split:
            self._write_split_file(title, info["year"], info["wikitext"], result)

    def _register_contributors(
        self,
        contributors: list[str],
        result: ParseResult,
        seen_authors: set[str],
    ) -> list[str]:
        from ob.api import author_add

        author_ids: list[str] = []
        for name in contributors:
            if name not in seen_authors:
                seen_authors.add(name)
                aid = author_add(
                    name=name, email=f"{name}@mediawiki", ob_dir=self.ob_dir
                )
                result.authors_registered += 1
                if aid not in author_ids:
                    author_ids.append(aid)
        return author_ids

    def _write_split_file(
        self, title: str, year: str, wikitext: str, result: ParseResult
    ) -> None:
        split_dir = self.ob_dir / ".ob" / "split"
        split_dir.mkdir(parents=True, exist_ok=True)

        safe_name = _safe_filename(title)
        filepath = split_dir / f"{safe_name}.xml"

        xml = (
            f"<mediawiki>\n<page>\n<title>{_xml_escape(title)}</title>\n"
            f"<revision><timestamp>{_xml_escape(year)}</timestamp>\n"
            f"<text>{_xml_escape(wikitext)}</text></revision>\n"
            f"</page>\n</mediawiki>\n"
        )
        filepath.write_text(xml, encoding="utf-8")
        result.split_files_created += 1


def _xml_escape(s: str) -> str:
    return s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
