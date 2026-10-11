#!/usr/bin/env python3
"""Keep every package's version in sync.

    python scripts/version.py            # print versions, fail if they differ
    python scripts/version.py 0.2.0      # set them all to 0.2.0
    python scripts/version.py --check-tag v0.2.0   # CI: fail unless all == tag

Files covered: the Cargo workspace version, the shrinkfit-cli dependency pin
on shrinkfit, the Python pyproject and __version__, and the npm package.json.
"""

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# (path, regex with one capture group around the version)
TARGETS = [
    ("Cargo.toml", r'(?ms)^\[workspace\.package\].*?^version = "([^"]+)"'),
    ("crates/shrinkfit-cli/Cargo.toml", r'shrinkfit = \{ path = "\.\./shrinkfit", version = "([^"]+)"'),
    ("bindings/python/pyproject.toml", r'(?m)^version = "([^"]+)"'),
    ("bindings/python/shrinkfit/__init__.py", r'(?m)^__version__ = "([^"]+)"'),
]
PACKAGE_JSON = "bindings/typescript/package.json"


def read_versions():
    found = {}
    for rel, pattern in TARGETS:
        m = re.search(pattern, (ROOT / rel).read_text(encoding="utf-8"))
        if not m:
            sys.exit(f"could not find a version in {rel}")
        found[rel] = m.group(1)
    found[PACKAGE_JSON] = json.loads((ROOT / PACKAGE_JSON).read_text(encoding="utf-8"))["version"]
    return found


def set_versions(new):
    for rel, pattern in TARGETS:
        path = ROOT / rel
        text = path.read_text(encoding="utf-8")
        m = re.search(pattern, text)
        text = text[: m.start(1)] + new + text[m.end(1):]
        path.write_text(text, encoding="utf-8", newline="\n")
    path = ROOT / PACKAGE_JSON
    text = path.read_text(encoding="utf-8")
    text = re.sub(r'("version":\s*")[^"]+(")', lambda m: m.group(1) + new + m.group(2), text, count=1)
    path.write_text(text, encoding="utf-8", newline="\n")


def main(argv):
    if len(argv) == 2 and argv[0] == "--check-tag":
        want = argv[1].removeprefix("v")
        bad = {k: v for k, v in read_versions().items() if v != want}
        if bad:
            sys.exit(f"tag says {want} but: {bad}")
        print(f"all versions match {want}")
    elif len(argv) == 1 and re.fullmatch(r"\d+\.\d+\.\d+([-+.][0-9A-Za-z.-]+)?", argv[0]):
        set_versions(argv[0])
        print(f"set all versions to {argv[0]} (remember to update Cargo.lock: cargo update -w)")
    elif not argv:
        versions = read_versions()
        for k, v in versions.items():
            print(f"{v:10} {k}")
        if len(set(versions.values())) != 1:
            sys.exit("versions differ")
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main(sys.argv[1:])
