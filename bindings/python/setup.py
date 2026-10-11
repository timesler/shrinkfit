"""Builds the Rust shared library and bundles it into the wheel.

Metadata lives in pyproject.toml; this file only adds the native build step.
Set SHRINKFIT_SKIP_CARGO=1 to bundle a library already placed in
shrinkfit/_native/ (e.g. by a cross-compiling CI job).
"""

import os
import shutil
import subprocess
import sys
from pathlib import Path

from setuptools import setup
from setuptools.command.build_py import build_py

try:
    from setuptools.command.bdist_wheel import bdist_wheel  # setuptools >= 70
except ImportError:  # pragma: no cover
    try:
        from wheel.bdist_wheel import bdist_wheel
    except ImportError:
        bdist_wheel = None

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]


def lib_name() -> str:
    if sys.platform == "win32":
        return "shrinkfit.dll"
    if sys.platform == "darwin":
        return "libshrinkfit.dylib"
    return "libshrinkfit.so"


class BuildPyWithNative(build_py):
    def run(self):
        native = HERE / "shrinkfit" / "_native"
        native.mkdir(exist_ok=True)
        if not os.environ.get("SHRINKFIT_SKIP_CARGO"):
            cmd = ["cargo", "build", "--release", "-p", "shrinkfit-ffi"]
            if os.environ.get("CARGO_NET_OFFLINE"):
                cmd.append("--offline")
            subprocess.check_call(cmd, cwd=ROOT)
            shutil.copy2(ROOT / "target" / "release" / lib_name(), native / lib_name())
        if not (native / lib_name()).exists():
            raise RuntimeError(f"native library missing: {native / lib_name()}")
        super().run()


cmdclass = {"build_py": BuildPyWithNative}

if bdist_wheel is not None:

    class PlatformWheel(bdist_wheel):
        """The wheel contains a native library, so it is platform-specific,
        but it does not use the Python C API, so it works on any Python 3."""

        def finalize_options(self):
            super().finalize_options()
            self.root_is_pure = False

        def get_tag(self):
            _, _, plat = super().get_tag()
            return "py3", "none", plat

    cmdclass["bdist_wheel"] = PlatformWheel

setup(cmdclass=cmdclass)
