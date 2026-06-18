__version__ = "0.2.0"

from ob.exceptions import ODError  # noqa: F401
from ob.api import init, author_add, register_section  # noqa: F401
from ob.track import track  # noqa: F401

# Provide backward-compatible source namespace (source.append / source.pop)
from ob.source import Source

source = Source()
