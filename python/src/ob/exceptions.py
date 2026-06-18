"""
Custom exception hierarchy for the ob library.

All exceptions in the ob library inherit from ODError.
"""


class ODError(Exception):
    """Base class for all ob library errors."""
    pass


class OBInitError(ODError):
    """Raised when .ob/ initialization fails."""
    pass


class OBNotInitializedError(ODError):
    """Raised when commands are run without ob initialization."""
    pass


class OBAuthorError(ODError):
    """Raised when author registration fails."""
    pass


class OBSectionError(ODError):
    """Raised when section registration fails."""
    pass


class OBSourceError(ODError):
    """Raised when source stack operations fail."""
    pass


class OBTrackError(ODError):
    """Raised when track() operations fail."""
    pass


class OBStorageError(ODError):
    """Raised when JSONL read/write operations fail."""
    pass


class OBRevokeError(ODError):
    """Raised when revoke operations fail."""
    pass


class OBPurgeError(ODError):
    """Raised when purge operations fail."""
    pass


class OBCleanError(ODError):
    """Raised when clean operations fail."""
    pass


class OBMergeError(ODError):
    """Raised when merge operations fail."""
    pass


__all__ = [
    "ODError",
    "OBInitError",
    "OBNotInitializedError",
    "OBAuthorError",
    "OBSectionError",
    "OBSourceError",
    "OBTrackError",
    "OBStorageError",
    "OBRevokeError",
    "OBPurgeError",
    "OBCleanError",
    "OBMergeError",
]
