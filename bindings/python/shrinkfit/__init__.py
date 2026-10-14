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
import urllib.error
import urllib.request
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
            lib.shrinkfit_model_request.restype = ctypes.c_void_p
            lib.shrinkfit_model_request.argtypes = lib.shrinkfit_shrink.argtypes
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


def _call(lib: ctypes.CDLL, fn: Any, text: bytes, opts: bytes) -> Dict[str, Any]:
    out_len = ctypes.c_size_t(0)
    ptr = fn(text, len(text), opts, len(opts), ctypes.byref(out_len))
    try:
        return json.loads(ctypes.string_at(ptr, out_len.value).decode("utf-8"))
    finally:
        lib.shrinkfit_free(ptr, out_len.value)


def _with_model_scores(lib: ctypes.CDLL, text: bytes, opts: bytes, send: Any = None) -> bytes:
    """Model scoring (``jev=...``): the core builds the request and reads the reply;
    this only moves the bytes, with the standard library, so it works in slim
    containers without curl. Any failure is passed back as ``jev_error`` and the
    core falls back to its built-in scoring."""
    envelope = _call(lib, lib.shrinkfit_model_request, text, opts)
    extra: Dict[str, str] = {}
    request = envelope.get("request") if envelope.get("ok") else None
    if not envelope.get("ok"):
        extra["jev_error"] = envelope.get("error", "model scoring is unavailable")
    elif request is not None and send is not None:
        try:
            extra["jev_reply"] = send(request)
        except Exception as e:  # the caller's transport: any failure just means no scores
            extra["jev_error"] = f"the request failed ({type(e).__name__})"
    elif request is not None:
        http = urllib.request.Request(
            request["url"],
            data=request["body"].encode("utf-8"),
            headers=request["headers"],
            method="POST",
        )
        try:
            with urllib.request.urlopen(http, timeout=request["timeout_ms"] / 1000) as response:
                extra["jev_reply"] = response.read().decode("utf-8")
        except urllib.error.HTTPError as e:
            extra["jev_error"] = f"the service answered HTTP {e.code}"
        except TimeoutError:
            extra["jev_error"] = "the request timed out"
        except OSError:  # URLError, connection errors
            extra["jev_error"] = "the request failed"
    if not extra:
        return opts
    return json.dumps({**json.loads(opts.decode("utf-8")), **extra}).encode("utf-8")


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
    decision model; see the README. The key defaults to ``OPENROUTER_API_KEY``. The
    core builds the request and uses the reply; the request is sent with ``urllib``
    unless you pass ``jev={"send": fn}``, where ``fn(request)`` receives a dict with
    ``url``, ``headers``, ``body`` and ``timeout_ms`` and returns the response body
    (raise on failure).
    """
    text = _to_text(data, input_format).encode("utf-8")
    send = None
    jev = options.get("jev")
    if isinstance(jev, dict) and "send" in jev:
        # A callable cannot go to the core as JSON; keep it here as the transport.
        jev = dict(jev)
        send = jev.pop("send")
        options = {**options, "jev": jev}
    opts = json.dumps(
        {"max_chars": int(max_chars), "format": format, "input_format": input_format, **options}
    ).encode("utf-8")
    lib = _get_lib()
    if options.get("jev") and "jev_reply" not in options and "jev_error" not in options:
        opts = _with_model_scores(lib, text, opts, send)
    envelope = _call(lib, lib.shrinkfit_shrink, text, opts)
    if not envelope.get("ok"):
        raise ShrinkError(envelope.get("error", "unknown error"))
    return ShrinkResult(text=envelope["text"], report=envelope["report"])


def shrink(data: Any, max_chars: int = 10_000, **kwargs: Any) -> str:
    """Like :func:`shrink_with_report` but returns only the text."""
    return shrink_with_report(data, max_chars, **kwargs).text


def native_version() -> str:
    """Version of the loaded native library."""
    return _get_lib().shrinkfit_version().decode()
