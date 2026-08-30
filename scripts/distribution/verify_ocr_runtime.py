#!/usr/bin/env python3
"""Verify a self-contained optional OCR plugin staged for one release target.

This is an artifact-structure gate, not an OCR accuracy or installed-runtime
smoke test or a base-release gate. It verifies the complete file inventory, pinned model bytes and
the absence of system Python/Tesseract dependencies declared by the bundle.
It never downloads dependencies and never executes files from the bundle.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path, PurePosixPath

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_PINNED_MODELS = ROOT / "distribution" / "ocr-models" / "manifest.toml"
RUNTIME_MANIFEST = "runtime-manifest.json"
ALLOWED_ROLES = {
    "engine",
    "sidecar",
    "runtime-library",
    "model-manifest",
    "model",
    "license-tesseract",
    "license-tessdata",
    "license-sidecar",
    "license-runtime",
}
REQUIRED_SINGLETON_ROLES = {
    "engine",
    "sidecar",
    "license-tesseract",
    "license-tessdata",
    "license-sidecar",
}
ALLOWED_SPDX = {"MIT", "Apache-2.0", "MIT OR Apache-2.0", "Apache-2.0 OR MIT"}


class VerificationError(ValueError):
    pass


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _safe_relative(value: object) -> PurePosixPath:
    if not isinstance(value, str) or not value or "\\" in value:
        raise VerificationError("runtime file paths must be non-empty POSIX paths")
    path = PurePosixPath(value)
    if path.is_absolute() or any(part in {"", ".", ".."} for part in path.parts):
        raise VerificationError(f"unsafe runtime file path: {value!r}")
    return path


def _load_json(path: Path) -> dict:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise VerificationError(f"cannot read {path}: {exc}") from exc
    if not isinstance(value, dict):
        raise VerificationError("runtime manifest must be a JSON object")
    return value


def _pinned_model_set(path: Path, name: str) -> dict:
    try:
        manifest = tomllib.loads(path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as exc:
        raise VerificationError(f"cannot read pinned model manifest: {exc}") from exc
    if manifest.get("schema") != "mpdf-ocr-model-manifest":
        raise VerificationError("unexpected pinned model manifest schema")
    matches = [entry for entry in manifest.get("model_set", []) if entry.get("name") == name]
    if len(matches) != 1:
        raise VerificationError(f"expected one pinned model set named {name!r}")
    selected = matches[0]
    if not selected.get("redistributable"):
        raise VerificationError(f"model set {name!r} is not redistributable")
    return selected


def _version_tuple(value: object) -> tuple[int, int, int]:
    if not isinstance(value, str):
        raise VerificationError("engine version must be a string")
    match = re.fullmatch(r"(\d+)\.(\d+)\.(\d+)(?:[-+].*)?", value)
    if not match:
        raise VerificationError(f"invalid engine version: {value!r}")
    return tuple(int(part) for part in match.groups())


def _validate_provenance(manifest: dict, target: str, *, allow_test_fixture: bool) -> None:
    """Validate portable provenance and dependency declarations.

    This is intentionally metadata-driven on non-macOS CI.  On macOS the
    staging tool records the install-name rewrites performed by
    ``install_name_tool``; absolute Homebrew/Python/Tesseract references are
    rejected on every platform, so a fixture cannot accidentally normalize a
    forbidden dependency into a release claim.
    """
    architecture = manifest.get("architecture")
    expected = {"aarch64-apple-darwin": "arm64", "x86_64-apple-darwin": "x86_64"}.get(target)
    if architecture is not None and architecture != expected:
        raise VerificationError(f"runtime architecture {architecture!r} does not match {expected!r}")
    provenance = manifest.get("provenance")
    if provenance is None:
        if allow_test_fixture:
            return
        raise VerificationError("runtime provenance is required for production bundles")
    if not isinstance(provenance, dict):
        raise VerificationError("runtime provenance must be an object")
    if provenance.get("target") != target:
        raise VerificationError("runtime provenance target mismatch")
    sources = provenance.get("sources")
    if not isinstance(sources, list):
        raise VerificationError("runtime provenance needs a sources list")
    source_roles = {item.get("role") for item in sources if isinstance(item, dict)}
    if not {"engine", "sidecar", "model-set"}.issubset(source_roles):
        raise VerificationError("runtime provenance is missing engine, sidecar, or model source")
    for item in sources:
        if not isinstance(item, dict) or not isinstance(item.get("license"), str) or not item["license"]:
            raise VerificationError("runtime provenance source is missing a license")
        if item.get("role") in {"engine", "sidecar", "runtime-library"} and not re.fullmatch(
            r"[0-9a-fA-F]{64}", str(item.get("sha256", ""))
        ):
            raise VerificationError("runtime provenance binary source is missing a SHA-256")
    policy = provenance.get("dependency_policy", {})
    if not isinstance(policy, dict):
        raise VerificationError("runtime dependency policy must be an object")
    configured_forbidden = policy.get("forbidden_prefixes", ())
    if not isinstance(configured_forbidden, (list, tuple)):
        raise VerificationError("runtime forbidden dependency prefixes must be a list")
    forbidden = tuple({"/opt/homebrew", "/usr/local", "/Library/Frameworks",
                      *configured_forbidden})
    observed = {key: value for key, value in provenance.items() if key != "dependency_policy"}

    def strings(value: object):
        if isinstance(value, str):
            yield value
        elif isinstance(value, dict):
            for item in value.values():
                yield from strings(item)
        elif isinstance(value, list):
            for item in value:
                yield from strings(item)

    if any(text.startswith(prefix) for text in strings(observed) for prefix in forbidden if prefix):
        raise VerificationError("runtime provenance contains a forbidden external dependency")
    # Absolute dependency observations are allowed only for Apple's system
    # locations.  This catches a path accidentally left in a custom manifest.
    for dependency in [*provenance.get("dependencies", []), *provenance.get("install_names", [])]:
        value = dependency.get("path") if isinstance(dependency, dict) else dependency
        if not isinstance(value, str) or not value.startswith("/"):
            if isinstance(value, str) and value.startswith("@rpath/"):
                owner = dependency.get("owner") if isinstance(dependency, dict) else None
                if isinstance(owner, str) and value == "@rpath/" + owner:
                    continue
                raise VerificationError(f"runtime has unresolved @rpath dependency: {value}")
            continue
        if not value.startswith(("/usr/lib/", "/System/Library/")):
            raise VerificationError(f"runtime has external absolute dependency: {value}")
    for rpath in provenance.get("rpaths", []):
        if isinstance(rpath, str) and rpath.startswith("/") and not rpath.startswith(("/usr/lib/", "/System/Library/")):
            raise VerificationError(f"runtime has external absolute rpath: {rpath}")
    for rewrite in provenance.get("install_name_rewrites", []):
        if not isinstance(rewrite, str):
            raise VerificationError("invalid Mach-O rewrite evidence")
        if rewrite.startswith("rpath "):
            if not rewrite.endswith(" -> removed"):
                raise VerificationError("invalid Mach-O rpath rewrite evidence")
            continue
        if " -> @loader_path/" not in rewrite:
            raise VerificationError("non-system Mach-O install name was not rewritten")
    if provenance.get("macho_required") and not isinstance(provenance.get("install_name_rewrites"), list):
        raise VerificationError("Mach-O runtime is missing install-name rewrite evidence")
    if not allow_test_fixture and (
        manifest.get("inspection_mode") != "mach-o"
        or manifest.get("post_rewrite_verified") is not True
        or manifest.get("ad_hoc_signed") is not True
        or provenance.get("inspection_mode") != "mach-o"
        or provenance.get("post_rewrite_verified") is not True
    ):
        raise VerificationError("runtime lacks successful post-rewrite Mach-O inspection")


def _verify_macho_host(root: Path, entries: list[dict], target: str) -> None:
    """Re-run Mach-O inspection when the structure gate runs on macOS.

    Staging evidence is still required on every host.  This second inspection
    prevents a hand-edited manifest from turning a non-relocatable bundle into
    a production pass on the target host.
    """
    if sys.platform != "darwin":
        return
    tools = ["lipo", "otool", "codesign"]
    missing = [tool for tool in tools if shutil.which(tool) is None]
    if missing:
        raise VerificationError("macOS Mach-O verifier tools missing: " + ", ".join(missing))
    expected = {"aarch64-apple-darwin": "arm64", "x86_64-apple-darwin": "x86_64"}[target]
    binary_paths: list[Path] = []
    for entry in entries:
        if entry.get("role") not in {"engine", "sidecar", "runtime-library"}:
            continue
        path = root.joinpath(*_safe_relative(entry.get("path")).parts)
        binary_paths.append(path)
        arches = subprocess.run(["lipo", "-archs", str(path)], capture_output=True, text=True)
        if arches.returncode or expected not in arches.stdout.split():
            raise VerificationError(f"Mach-O architecture mismatch: {entry.get('path')}")
        signature = subprocess.run(["codesign", "--verify", "--strict", str(path)],
                                   capture_output=True, text=True)
        if signature.returncode:
            raise VerificationError(f"invalid Mach-O code signature: {entry.get('path')}")
        deps = subprocess.run(["otool", "-L", str(path)], capture_output=True, text=True)
        load = subprocess.run(["otool", "-l", str(path)], capture_output=True, text=True)
        if deps.returncode or load.returncode:
            raise VerificationError(f"cannot inspect Mach-O load commands: {entry.get('path')}")
        dependency_lines = [line.strip().split(" ", 1)[0]
                            for line in deps.stdout.splitlines()[1:] if line.strip()]
        for dependency in dependency_lines:
            if path.suffix == ".dylib" and dependency == "@rpath/" + path.name:
                continue  # the dylib's own install name
            if dependency.startswith(("/usr/lib/", "/System/Library/")):
                continue
            if dependency.startswith("@rpath/"):
                raise VerificationError(f"unresolved @rpath dependency: {entry.get('path')}")
            if dependency.startswith("@loader_path/"):
                resolved = (path.parent / dependency.removeprefix("@loader_path/")).resolve()
                if root not in resolved.parents or not resolved.is_file():
                    raise VerificationError(f"Mach-O dependency escapes runtime: {entry.get('path')}")
                continue
            raise VerificationError(f"non-relocatable Mach-O dependency: {entry.get('path')}: {dependency}")
        lines = load.stdout.splitlines()
        for index, line in enumerate(lines):
            if line.strip() != "cmd LC_RPATH":
                continue
            for candidate in lines[index + 1:index + 5]:
                if candidate.strip().startswith("path "):
                    rpath = candidate.strip().split(" ", 2)[1]
                    if rpath.startswith("/") and not rpath.startswith(("/usr/lib/", "/System/Library/")):
                        raise VerificationError(f"external Mach-O LC_RPATH: {rpath}")
                    break


def verify(runtime_root: Path, target: str, pinned_models: Path = DEFAULT_PINNED_MODELS,
           *, expected_release: str = "0.1.0-rc.3", allow_test_fixture: bool = False) -> dict:
    if re.fullmatch(r"[A-Za-z0-9_.-]+", target) is None:
        raise VerificationError("invalid release target identifier")
    root = runtime_root.resolve(strict=True)
    if not root.is_dir() or runtime_root.is_symlink():
        raise VerificationError("OCR runtime root must be a real directory")
    manifest_path = root / RUNTIME_MANIFEST
    if not manifest_path.is_file() or manifest_path.is_symlink():
        raise VerificationError(f"missing real file {RUNTIME_MANIFEST}")
    manifest = _load_json(manifest_path)
    if manifest.get("schema") != "mpdf-ocr-runtime-bundle" or manifest.get("schema_version") != "1.0":
        raise VerificationError("unexpected OCR runtime manifest protocol")
    if manifest.get("release") != expected_release or manifest.get("target") != target:
        raise VerificationError("OCR runtime release or target mismatch")
    if manifest.get("engine") != "tesseract" or manifest.get("sidecar_protocol") != "mpdf-ocr/0.1":
        raise VerificationError("unexpected OCR engine or sidecar protocol")
    if "sidecar_version" in manifest and (not isinstance(manifest["sidecar_version"], str) or not manifest["sidecar_version"].strip()):
        raise VerificationError("sidecar version must be a non-empty string")
    if manifest.get("requires_system_python") is not False:
        raise VerificationError("runtime must not require system Python")
    if manifest.get("requires_system_tesseract") is not False:
        raise VerificationError("runtime must not require system Tesseract")
    _validate_provenance(manifest, target, allow_test_fixture=allow_test_fixture)

    selected = _pinned_model_set(pinned_models, str(manifest.get("model_set", "")))
    if manifest.get("model_set_version") != selected.get("version"):
        raise VerificationError("model set version does not match pinned manifest")
    if _version_tuple(manifest.get("engine_version")) < _version_tuple(selected.get("engine_min_version")):
        raise VerificationError("bundled Tesseract is older than the pinned minimum")

    entries = manifest.get("files")
    if not isinstance(entries, list) or not entries:
        raise VerificationError("runtime manifest needs a non-empty files list")
    if target.endswith("-apple-darwin"):
        if not allow_test_fixture and manifest.get("inspection_mode") != "mach-o":
            raise VerificationError("macOS runtime requires successful Mach-O inspection")
        for entry in entries:
            if entry.get("role") in {"engine", "sidecar", "runtime-library"}:
                candidate = root.joinpath(*_safe_relative(entry.get("path")).parts)
                if not allow_test_fixture and candidate.read_bytes()[:4] not in {
                    b"\xfe\xed\xfa\xce", b"\xce\xfa\xed\xfe", b"\xfe\xed\xfa\xcf",
                    b"\xcf\xfa\xed\xfe", b"\xca\xfe\xba\xbe", b"\xbe\xba\xfe\xca",
                }:
                    raise VerificationError(f"macOS runtime entry is not Mach-O: {entry.get('path')}")
    listed: set[str] = set()
    role_counts: dict[str, int] = {}
    model_entries: dict[str, dict] = {}
    for entry in entries:
        if not isinstance(entry, dict):
            raise VerificationError("each runtime file entry must be an object")
        relative = _safe_relative(entry.get("path"))
        relative_text = relative.as_posix()
        if relative_text == RUNTIME_MANIFEST or relative_text in listed:
            raise VerificationError(f"duplicate or reserved runtime path: {relative_text}")
        listed.add(relative_text)
        role = entry.get("role")
        if role not in ALLOWED_ROLES:
            raise VerificationError(f"unsupported runtime file role: {role!r}")
        if role == "license-sidecar" and not allow_test_fixture and entry.get("license") not in ALLOWED_SPDX:
            raise VerificationError("sidecar license expression is missing or unsupported")
        if (role in {"engine", "sidecar", "model", "runtime-library"}
                and not allow_test_fixture
                and not isinstance(entry.get("license"), str)):
            raise VerificationError(f"runtime entry license is missing: {relative_text}")
        role_counts[role] = role_counts.get(role, 0) + 1
        path = root.joinpath(*relative.parts)
        try:
            resolved = path.resolve(strict=True)
        except OSError as exc:
            raise VerificationError(f"missing runtime file {relative_text}: {exc}") from exc
        if path.is_symlink() or not resolved.is_file() or root not in resolved.parents:
            raise VerificationError(f"runtime entry is not a contained real file: {relative_text}")
        size = path.stat().st_size
        digest = sha256(path)
        if entry.get("size_bytes") != size or entry.get("sha256") != digest:
            raise VerificationError(f"runtime file digest mismatch: {relative_text}")
        if role in {"engine", "sidecar"} and os.name != "nt" and not os.access(path, os.X_OK):
            raise VerificationError(f"runtime executable lacks execute permission: {relative_text}")
        if entry.get("architecture") is not None:
            expected_arch = {"aarch64-apple-darwin": "arm64", "x86_64-apple-darwin": "x86_64"}.get(target)
            architectures = str(entry["architecture"]).split("+")
            if expected_arch and expected_arch not in architectures:
                raise VerificationError(f"runtime entry architecture mismatch: {relative_text}")
        if role == "model":
            language = entry.get("language")
            if not isinstance(language, str) or language in model_entries:
                raise VerificationError("model languages must be unique strings")
            model_entries[language] = entry

    if target.endswith("-apple-darwin") and not allow_test_fixture:
        _verify_macho_host(root, entries, target)

    for role in REQUIRED_SINGLETON_ROLES:
        if role_counts.get(role) != 1:
            raise VerificationError(f"runtime needs exactly one {role!r} file")

    pinned_files = {spec["language"]: spec for spec in selected.get("file", [])}
    if set(model_entries) != set(pinned_files):
        raise VerificationError("runtime model languages do not exactly match the pinned model set")
    for language, spec in pinned_files.items():
        entry = model_entries[language]
        if (PurePosixPath(str(entry["path"])).name != spec["filename"]
                or entry["sha256"] != spec["sha256"]
                or entry["size_bytes"] != spec["size_bytes"]):
            raise VerificationError(f"bundled {language} model does not match pinned bytes")

    actual = {
        path.relative_to(root).as_posix()
        for path in root.rglob("*")
        if path.is_file() and path != manifest_path
    }
    symlinks = [path.relative_to(root).as_posix() for path in root.rglob("*") if path.is_symlink()]
    if symlinks:
        raise VerificationError(f"runtime contains symbolic links: {sorted(symlinks)}")
    if actual != listed:
        raise VerificationError(
            f"runtime inventory mismatch; unlisted={sorted(actual - listed)} missing={sorted(listed - actual)}"
        )
    return {
        "schema": "mpdf-ocr-runtime-structure-evidence",
        "schema_version": "1.0",
        "release": manifest["release"],
        "target": target,
        "runtime_manifest_sha256": sha256(manifest_path),
        "model_set": manifest["model_set"],
        "engine_version": manifest["engine_version"],
        "file_count": len(listed),
        "status": "pass_structure_only",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runtime-root", required=True, type=Path)
    parser.add_argument("--target", required=True)
    parser.add_argument("--pinned-models", type=Path, default=DEFAULT_PINNED_MODELS)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()
    try:
        evidence = verify(args.runtime_root, args.target, args.pinned_models)
    except VerificationError as exc:
        print(f"OCR runtime verification failed: {exc}", file=sys.stderr)
        return 1
    if args.json:
        print(json.dumps(evidence, indent=2, sort_keys=True))
    else:
        print(
            f"verified {evidence['file_count']} OCR runtime files for {evidence['target']} "
            "(structure only; installed OCR smoke still required)"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
