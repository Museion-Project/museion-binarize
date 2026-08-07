#!/usr/bin/env python3
"""Fetches, verifies, and stages the pinned PDFium library for one target.

Reads distribution/pdfium/manifest.toml, downloads the pinned archive for
the requested target triple, verifies its SHA-256 against the manifest,
extracts the library file, verifies *its* SHA-256 too, and copies it into
a gitignored staging directory that packaging picks up from.

Usage:
    python3 scripts/distribution/fetch_pdfium.py <target-triple> [--dest DIR]

Exits non-zero and prints a clear error on any checksum mismatch, missing
manifest entry, or download failure. Never falls back to an unverified
copy. Never writes "latest" anywhere — every URL comes from the pinned
manifest.
"""

from __future__ import annotations

import argparse
import hashlib
import shutil
import sys
import tarfile
import tomllib
import urllib.request
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
MANIFEST_PATH = REPO_ROOT / "distribution" / "pdfium" / "manifest.toml"
DEFAULT_DEST = REPO_ROOT / "target" / "distribution" / "pdfium"


def sha256_of(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_manifest() -> dict:
    with MANIFEST_PATH.open("rb") as f:
        return tomllib.load(f)


def find_asset(manifest: dict, target_triple: str) -> dict:
    for asset in manifest.get("asset", []):
        if asset["target_triple"] == target_triple:
            return asset
    known = ", ".join(a["target_triple"] for a in manifest.get("asset", []))
    raise SystemExit(
        f"error: no PDFium asset pinned for target '{target_triple}' in "
        f"{MANIFEST_PATH}. Known targets: {known}"
    )


def download(url: str, dest: Path) -> None:
    print(f"downloading {url}", file=sys.stderr)
    with urllib.request.urlopen(url, timeout=120) as response, dest.open("wb") as out:
        shutil.copyfileobj(response, out)


def fetch_and_verify(target_triple: str, dest_root: Path) -> Path:
    manifest = load_manifest()
    asset = find_asset(manifest, target_triple)

    dest_root.mkdir(parents=True, exist_ok=True)
    target_dir = dest_root / target_triple
    target_dir.mkdir(parents=True, exist_ok=True)

    archive_path = target_dir / asset["archive_filename"]
    download(asset["archive_url"], archive_path)

    actual_archive_sha = sha256_of(archive_path)
    expected_archive_sha = asset["archive_sha256"]
    if actual_archive_sha != expected_archive_sha:
        archive_path.unlink(missing_ok=True)
        raise SystemExit(
            f"error: archive checksum mismatch for {asset['archive_filename']}\n"
            f"  expected: {expected_archive_sha}\n"
            f"  actual:   {actual_archive_sha}\n"
            "refusing to use this download."
        )
    print(f"archive checksum verified: {actual_archive_sha}", file=sys.stderr)

    extract_dir = target_dir / "extracted"
    if extract_dir.exists():
        shutil.rmtree(extract_dir)
    extract_dir.mkdir(parents=True)

    with tarfile.open(archive_path) as tar:
        _safe_extract(tar, extract_dir)

    extracted_library = extract_dir / asset["archive_library_path"]
    if not extracted_library.is_file():
        raise SystemExit(
            f"error: expected library at {asset['archive_library_path']} inside "
            f"{asset['archive_filename']}, but it was not found after extraction"
        )

    actual_library_sha = sha256_of(extracted_library)
    expected_library_sha = asset["library_sha256"]
    if actual_library_sha != expected_library_sha:
        raise SystemExit(
            f"error: extracted library checksum mismatch for "
            f"{asset['library_filename']}\n"
            f"  expected: {expected_library_sha}\n"
            f"  actual:   {actual_library_sha}\n"
            "refusing to stage this library."
        )
    print(f"library checksum verified: {actual_library_sha}", file=sys.stderr)

    staged_library = target_dir / asset["library_filename"]
    shutil.copy2(extracted_library, staged_library)

    print(f"staged {asset['library_filename']} -> {staged_library}", file=sys.stderr)
    return staged_library


def _safe_extract(tar: tarfile.TarFile, dest: Path) -> None:
    """Extracts `tar` into `dest`, rejecting any member that would escape
    it via a `../` path or an absolute path — a tar archive is untrusted
    input even when its own checksum has already been verified, since the
    checksum only proves the bytes are the ones that were pinned, not
    that those bytes are safe to extract naively."""
    dest_resolved = dest.resolve()
    for member in tar.getmembers():
        member_path = (dest / member.name).resolve()
        if not str(member_path).startswith(str(dest_resolved) + "/") and member_path != dest_resolved:
            raise SystemExit(f"error: archive member '{member.name}' escapes the extraction directory")
    tar.extractall(dest)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("target_triple", help="e.g. aarch64-apple-darwin")
    parser.add_argument(
        "--dest",
        type=Path,
        default=DEFAULT_DEST,
        help=f"staging root (default: {DEFAULT_DEST})",
    )
    args = parser.parse_args()
    fetch_and_verify(args.target_triple, args.dest)


if __name__ == "__main__":
    main()
