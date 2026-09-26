"""Dependency-free PEP 660 backend for the local editable product plugin."""

from __future__ import annotations

import base64
import hashlib
from pathlib import Path
import zipfile


DIST_INFO = "wptrunner_raikiri-0.1.0.dist-info"
WHEEL_NAME = "wptrunner_raikiri-0.1.0-py3-none-any.whl"


def get_requires_for_build_wheel(config_settings=None):
    return []


def get_requires_for_build_editable(config_settings=None):
    return []


def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
    return _build_editable_wheel(wheel_directory)


def build_editable(wheel_directory, config_settings=None, metadata_directory=None):
    return _build_editable_wheel(wheel_directory)


def _build_editable_wheel(wheel_directory):
    source_root = Path.cwd().resolve()
    files = {
        "_wptrunner_raikiri_editable.pth": f"{source_root}\n".encode(),
        f"{DIST_INFO}/METADATA": (
            "Metadata-Version: 2.1\n"
            "Name: wptrunner-raikiri\n"
            "Version: 0.1.0\n"
            "Summary: External upstream wptrunner product for Raikiri\n"
        ).encode(),
        f"{DIST_INFO}/WHEEL": (
            "Wheel-Version: 1.0\n"
            "Generator: wptrunner-raikiri-build-backend\n"
            "Root-Is-Purelib: true\n"
            "Tag: py3-none-any\n"
        ).encode(),
        f"{DIST_INFO}/entry_points.txt": (
            "[wptrunner.products]\n"
            "raikiri = wptrunner_raikiri:get_product\n"
        ).encode(),
    }
    records = []
    for name, data in files.items():
        digest = base64.urlsafe_b64encode(hashlib.sha256(data).digest()).rstrip(b"=")
        records.append(f"{name},sha256={digest.decode()},{len(data)}")
    record_name = f"{DIST_INFO}/RECORD"
    records.append(f"{record_name},,")
    files[record_name] = ("\n".join(records) + "\n").encode()

    wheel_path = Path(wheel_directory) / WHEEL_NAME
    with zipfile.ZipFile(wheel_path, "w", compression=zipfile.ZIP_DEFLATED) as wheel:
        for name, data in files.items():
            wheel.writestr(name, data)
    return WHEEL_NAME
