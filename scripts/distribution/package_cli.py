#!/usr/bin/env python3
"""Assembles one standalone CLI release archive: the CLI executable next
to its matching PDFium library, license files, and a short README — see
docs/pdfium-bundling.md, "CLI distribution." The CLI's own resolver
(`mpdf_core::pdfium_backend::resolve_library`) already
searches next to its own executable (`LibrarySource::ExecutableAdjacent`),
so this layout needs no new resolution code — only correct packaging.

Usage:
    python3 scripts/distribution/package_cli.py \\
        --target-triple aarch64-apple-darwin \\
        --binary target/release/mpdf \\
        --pdfium-library target/distribution/pdfium/aarch64-apple-darwin/libpdfium.dylib \\
        --version 0.1.0 \\
        --out-dir /tmp/cli-release
"""

from __future__ import annotations

import argparse
import shutil
import sys
import tarfile
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import naming  # noqa: E402

REPO_ROOT = Path(__file__).resolve().parents[2]

README_TEMPLATE = """M PDF Processor CLI {version} ({target_triple})

This archive is self-contained: the CLI executable and its matching
PDFium library are in the same directory, and the CLI finds it
automatically without any environment variable or installation step.

Usage:
    {exe_name} --help
    {exe_name} info
    {exe_name} inspect some.pdf
    {exe_name} process some.pdf --output some-binarized.pdf

See LICENSE-MIT, LICENSE-APACHE, and THIRD-PARTY-NOTICES.md for licensing,
including the bundled PDFium library's own license and provenance.

Project: https://github.com/Museion-Project/museion-binarize
"""


def library_filename_for(target_triple: str) -> str:
    if "darwin" in target_triple:
        return "libpdfium.dylib"
    if "windows" in target_triple:
        return "pdfium.dll"
    return "libpdfium.so"


def package(
    target_triple: str,
    binary_path: Path,
    pdfium_library_path: Path,
    version: str,
    out_dir: Path,
) -> Path:
    is_windows = "windows" in target_triple
    exe_name = "mpdf.exe" if is_windows else "mpdf"
    lib_name = library_filename_for(target_triple)

    if pdfium_library_path.name != lib_name:
        raise SystemExit(
            f"error: expected PDFium library named '{lib_name}' for target "
            f"'{target_triple}', got '{pdfium_library_path.name}'"
        )

    staging_name = f"mpdf-cli-{version}-{'-'.join(naming.os_arch_label(target_triple))}"
    staging_dir = out_dir / staging_name
    if staging_dir.exists():
        shutil.rmtree(staging_dir)
    staging_dir.mkdir(parents=True)

    shutil.copy2(binary_path, staging_dir / exe_name)
    if not is_windows:
        (staging_dir / exe_name).chmod(0o755)
    shutil.copy2(pdfium_library_path, staging_dir / lib_name)

    for license_file in ("LICENSE-MIT", "LICENSE-APACHE"):
        shutil.copy2(REPO_ROOT / license_file, staging_dir / license_file)
    shutil.copy2(
        REPO_ROOT / "THIRD_PARTY_LICENSES.md", staging_dir / "THIRD-PARTY-NOTICES.md"
    )
    (staging_dir / "README.txt").write_text(
        README_TEMPLATE.format(version=version, target_triple=target_triple, exe_name=exe_name)
    )

    if is_windows:
        archive_name = naming.cli_archive_name(version, target_triple, "zip")
        archive_path = out_dir / archive_name
        with zipfile.ZipFile(archive_path, "w", zipfile.ZIP_DEFLATED) as zf:
            for path in sorted(staging_dir.rglob("*")):
                zf.write(path, arcname=f"{staging_name}/{path.relative_to(staging_dir)}")
    else:
        archive_name = naming.cli_archive_name(version, target_triple, "tar.gz")
        archive_path = out_dir / archive_name
        with tarfile.open(archive_path, "w:gz") as tar:
            tar.add(staging_dir, arcname=staging_name)

    shutil.rmtree(staging_dir)
    print(f"packaged {archive_path}", file=sys.stderr)
    return archive_path


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target-triple", required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--pdfium-library", type=Path, required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--out-dir", type=Path, required=True)
    args = parser.parse_args()
    args.out_dir.mkdir(parents=True, exist_ok=True)
    package(args.target_triple, args.binary, args.pdfium_library, args.version, args.out_dir)


if __name__ == "__main__":
    main()
