"""Deterministic release-artifact naming convention, shared by every
distribution script so exactly one place decides what a filename looks
like. See docs/releasing.md, "Artifact naming."

Convention:

    mpdf-<version>-<os>-<arch>.<ext>          (desktop)
    mpdf-cli-<version>-<os>-<arch>.<ext>      (CLI archive)

No workflow-run-number, timestamp, or commit hash in the filename —
those live in the release manifest instead, so the same version+target
always produces the same filename.
"""

from __future__ import annotations

TARGET_LABELS = {
    "aarch64-apple-darwin": ("macos", "arm64"),
    "x86_64-apple-darwin": ("macos", "x64"),
    "x86_64-pc-windows-msvc": ("windows", "x64"),
    "x86_64-unknown-linux-gnu": ("linux", "x86_64"),
}


def os_arch_label(target_triple: str) -> tuple[str, str]:
    if target_triple not in TARGET_LABELS:
        raise ValueError(f"no naming convention defined for target '{target_triple}'")
    return TARGET_LABELS[target_triple]


def desktop_artifact_name(version: str, target_triple: str, ext: str) -> str:
    os_label, arch_label = os_arch_label(target_triple)
    # Intent 8.46 BP1: rename from 0.2; retain historical release filenames.
    prefix = "mpdf" if tuple(map(int, version.split("-")[0].split(".")[:2])) < (0, 2) else "Museion-PDF"
    return f"{prefix}-{version}-{os_label}-{arch_label}.{ext}"


def cli_archive_name(version: str, target_triple: str, ext: str) -> str:
    os_label, arch_label = os_arch_label(target_triple)
    return f"mpdf-cli-{version}-{os_label}-{arch_label}.{ext}"
