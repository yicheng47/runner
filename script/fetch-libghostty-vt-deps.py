#!/usr/bin/env python3
"""Download and verify the pinned libghostty-vt Zig package cache for offline builds."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import sys
import tarfile
import tempfile
import urllib.request
from pathlib import Path


def repo_root() -> Path:
    return Path(__file__).resolve().parent.parent


def load_metadata(root: Path) -> dict[str, str]:
    meta_path = root / "vendor" / "libghostty-vt.vendor.json"
    return json.loads(meta_path.read_text(encoding="utf-8"))


def default_cache_dir(meta: dict[str, str]) -> Path:
    slug = f"{meta['version']}-{meta['deps_sha256'][:8]}"
    if sys.platform == "win32":
        base = Path(os.environ.get("LOCALAPPDATA") or (Path.home() / "AppData" / "Local"))
    else:
        base = Path(os.environ.get("XDG_CACHE_HOME") or (Path.home() / ".cache"))
    return base / "runner" / "libghostty-vt-deps" / slug


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def ensure_deps(dest: Path, meta: dict[str, str], force: bool = False) -> Path:
    marker = dest / ".complete"
    if not force and marker.is_file():
        return dest

    url = meta["deps_url"]
    expected_sha = meta["deps_sha256"]
    extracted_name = meta["deps_extracted_dir"]

    dest.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=dest.parent, prefix=".libghostty-vt-deps-") as tmp:
        tmp_path = Path(tmp)
        archive = tmp_path / "deps.tar.gz"
        print(f"Downloading {url} ...", file=sys.stderr)
        request = urllib.request.Request(
            url, headers={"User-Agent": "runner-fetch-libghostty-vt-deps/1.0"}
        )
        with urllib.request.urlopen(request) as response, archive.open("wb") as out:
            shutil.copyfileobj(response, out)

        actual_sha = sha256_file(archive)
        if actual_sha != expected_sha:
            raise RuntimeError(
                f"SHA-256 mismatch for {url}: got {actual_sha}, expected {expected_sha}"
            )

        extract_root = tmp_path / "extract"
        extract_root.mkdir()
        with tarfile.open(archive, "r:gz") as tar:
            if hasattr(tarfile, "data_filter"):
                tar.extractall(extract_root, filter="data")
            else:
                tar.extractall(extract_root)

        unpacked = extract_root / extracted_name
        if not unpacked.is_dir():
            raise RuntimeError(f"expected extracted directory {extracted_name} in {url}")

        (unpacked / ".complete").write_text(f"{expected_sha}\n", encoding="utf-8")

        if dest.exists():
            shutil.rmtree(dest)
        shutil.move(str(unpacked), str(dest))

    return dest


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Fetch the pinned libghostty-vt Zig --system dependency directory."
    )
    parser.add_argument(
        "--output",
        type=Path,
        help="Destination directory (defaults to the shared per-user Runner cache).",
    )
    parser.add_argument(
        "--print-path",
        action="store_true",
        help="Only print the resolved cache directory path without downloading.",
    )
    parser.add_argument(
        "--force",
        action="store_true",
        help="Re-download and replace an existing cache directory.",
    )
    args = parser.parse_args()

    meta = load_metadata(repo_root())
    dest = args.output.resolve() if args.output else default_cache_dir(meta)
    if args.print_path:
        print(dest)
        return 0

    ensure_deps(dest, meta, force=args.force)
    print(dest)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
