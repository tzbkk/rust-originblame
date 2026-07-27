"""OriginBlame — record- and token-level data provenance for AI training datasets.

Public API: init(), author_add(), register_section(), track(), source.
Three-layer model: authors ← sections ← document-index.
"""

__version__ = "0.2.0"

from ob.exceptions import ODError  # noqa: F401
from ob.api import init, author_add, register_section  # noqa: F401
from ob.track import track  # noqa: F401

# Provide backward-compatible source namespace (source.append / source.pop)
from ob.source import Source

source = Source()

__all__ = ["init", "author_add", "register_section", "track", "source", "ODError"]

