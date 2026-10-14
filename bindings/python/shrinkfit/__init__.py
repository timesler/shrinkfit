"""shrinkfit: shrink JSON/YAML payloads to fit a character budget while
preserving their schema and structure.

>>> from shrinkfit import shrink
>>> shrink({"items": list(range(1000)), "total": 1000}, max_chars=80, indent=0)
'{"items":[0,1,2,3,4,5,6,7,8,9,10,11,12,"…[986 items omitted]",999],"total":1000}'

This module is a thin ctypes wrapper around the Rust core's C ABI. Options
are passed through as JSON, so every core option works here unchanged.
"""

from __future__ import annotations

import ctypes
import json
import os
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Dict, Optional

__all__ = ["shrink", "shrink_with_report", "ShrinkResult", "ShrinkError", "__version__"]

__version__ = "0.0.0"


class ShrinkError(ValueError):
    """Raised for invalid input or options."""


@dataclass(frozen=True)
class ShrinkResult:
    text: str
    """The shrunk document, rendered in the requested format."""
    report: Dict[str, Any]
    """What was cut: input_chars, output_chars, fits, items_omitted, ..."""


def _lib_names():
    if sys.platform == "win32":
        return ["shrinkfit.dll"]
    if sys.platform == "darwin":
        return ["libshrinkfit.dylib"]
    return ["libshrinkfit.so"]


def _load() -> ctypes.CDLL:
    override = os.environ.get("SHRINKFIT_LIBRARY")
    candidates = [Path(override)] if override else []
    here = Path(__file__).resolve().parent
    for name in _lib_names():
        candidates.append(here / "_native" / name)
        # Development fallback: a cargo build in the repository.
        for profile in ("release", "debug"):
            candidates.append(here.parents[2] / "target" / profile / name)
    for path in candidates:
        if path.is_file():
            lib = ctypes.CDLL(str(path))
            lib.shrinkfit_shrink.restype = ctypes.c_void_p
            lib.shrinkfit_shrink.argtypes = [
                ctypes.c_char_p, ctypes.c_size_t,
                ctypes.c_char_p, ctypes.c_size_t,
                ctypes.POINTER(ctypes.c_size_t),
            ]
            lib.shrinkfit_free.restype = None
            lib.shrinkfit_free.argtypes = [ctypes.c_void_p, ctypes.c_size_t]
            lib.shrinkfit_version.restype = ctypes.c_char_p
            return lib
    raise ImportError(
        "shrinkfit native library not found; build it with "
        "`cargo build --release -p shrinkfit-ffi` or set SHRINKFIT_LIBRARY"
    )


_lib: Optional[ctypes.CDLL] = None


def _get_lib() -> ctypes.CDLL:
    global _lib
    if _lib is None:
        _lib = _load()
    return _lib


def _to_text(data: Any, input_format: str) -> str:
    if isinstance(data, (bytes, bytearray)):
        data = bytes(data).decode("utf-8")
    if isinstance(data, str):
        return data
    # Python objects are always handed over as JSON.
    return json.dumps(data, ensure_ascii=False, default=str)


def shrink_with_report(
    data: Any,
    max_chars: int = 10_000,
    *,
    format: str = "auto",
    input_format: str = "auto",
    **options: Any,
) -> ShrinkResult:
    """Shrink ``data`` to at most ``max_chars`` characters.

    ``data`` may be a Python object (dict, list, ...), or a ``str``/``bytes``
    holding a serialized document: JSON by default, or YAML with
    ``input_format="yaml"`` (parsed natively by the core). Non-JSON values such as
    datetimes are converted with ``str()``.

    ``format`` is the output format: ``"auto"`` (default, same as the input),
    ``"json"`` or ``"yaml"``; ``max_chars`` defaults to 10000, ``indent`` to 2. Other keyword
    options are passed to the core unchanged: ``indent``,
    ``string_head_chars``, ``string_section_chars``,
    ``string_min_section_chars``, ``keep_last_item``, ``max_samples``,
    ``min_partial_chars``.

    ``jev=True`` (or ``{"api_key": ..., "tool": ...}``) blends in scores from a hosted
    decision model; the core sends the request itself. See the README. The key
    defaults to ``OPENROUTER_API_KEY``.
    """
    text = _to_text(data, input_format).encode("utf-8")
    opts = json.dumps(
        {"max_chars": int(max_chars), "format": format, "input_format": input_format, **options}
    ).encode("utf-8")
    lib = _get_lib()
    out_len = ctypes.c_size_t(0)
    ptr = lib.shrinkfit_shrink(text, len(text), opts, len(opts), ctypes.byref(out_len))
    try:
        envelope = json.loads(ctypes.string_at(ptr, out_len.value).decode("utf-8"))
    finally:
        lib.shrinkfit_free(ptr, out_len.value)
    if not envelope.get("ok"):
        raise ShrinkError(envelope.get("error", "unknown error"))
    return ShrinkResult(text=envelope["text"], report=envelope["report"])


def shrink(data: Any, max_chars: int = 10_000, **kwargs: Any) -> str:
    """Like :func:`shrink_with_report` but returns only the text."""
    return shrink_with_report(data, max_chars, **kwargs).text


def native_version() -> str:
    """Version of the loaded native library."""
    return _get_lib().shrinkfit_version().decode()
