#!/usr/bin/env python3
"""Collect a pinned Homebrew Tesseract Mach-O closure as real input files.

Homebrew is a build input only. The output contains copied regular files,
component metadata and license texts; no symlink or Homebrew path is written
to the manifest consumed by the release stager.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import subprocess
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_COMPONENTS = ROOT / "distribution/ocr-runtime/macos-arm64-components.toml"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _otool(path: Path) -> list[str]:
    result = subprocess.run(["otool", "-L", str(path)], capture_output=True, text=True, check=True)
    return [line.strip().split(" ", 1)[0] for line in result.stdout.splitlines()[1:] if line.strip()]


def _rpaths(path: Path) -> list[str]:
    output = subprocess.check_output(["otool", "-l", str(path)], text=True)
    lines = output.splitlines()
    values: list[str] = []
    for index, line in enumerate(lines):
        if line.strip() != "cmd LC_RPATH":
            continue
        for candidate in lines[index + 1:index + 5]:
            if candidate.strip().startswith("path "):
                values.append(candidate.strip().split(" ", 2)[1])
                break
    return values


def _expand(value: str, owner: Path, executable: Path) -> Path:
    if value.startswith("@loader_path/"):
        return owner.parent / value.removeprefix("@loader_path/")
    if value.startswith("@executable_path/"):
        return executable.parent / value.removeprefix("@executable_path/")
    return Path(value)


def _resolve_dependency(value: str, owner: Path, executable: Path) -> Path:
    if value.startswith("@rpath/"):
        suffix = value.removeprefix("@rpath/")
        candidates = [_expand(rpath, owner, executable) / suffix for rpath in _rpaths(owner)]
        matches = [candidate.resolve(strict=True) for candidate in candidates if candidate.exists()]
        if len(matches) != 1:
            raise ValueError(f"cannot uniquely resolve {value} from {owner.name}")
        return matches[0]
    candidate = _expand(value, owner, executable)
    return candidate.resolve(strict=True)


def _load_components(path: Path) -> tuple[str, list[dict]]:
    data = tomllib.loads(path.read_text(encoding="utf-8"))
    if data.get("schema") != "mpdf-macos-ocr-components" or data.get("schema_version") != "1.0":
        raise ValueError("unexpected macOS OCR component manifest")
    components = data.get("component")
    if not isinstance(components, list) or not components:
        raise ValueError("component manifest is empty")
    return str(data.get("target")), components


def collect(engine: Path, output: Path, components_path: Path = DEFAULT_COMPONENTS) -> Path:
    target, specs = _load_components(components_path)
    if target != "aarch64-apple-darwin":
        raise ValueError("component manifest target mismatch")
    roots: list[tuple[Path, dict]] = []
    for spec in specs:
        formula = str(spec.get("formula", ""))
        prefix = Path(subprocess.check_output(["brew", "--prefix", formula], text=True).strip()).resolve()
        if prefix.name != str(spec.get("version")):
            raise ValueError(f"installed {formula} version {prefix.name!r} is not pinned {spec.get('version')!r}")
        if not isinstance(spec.get("license"), str) or not spec["license"]:
            raise ValueError(f"component {formula} has no license expression")
        roots.append((prefix, spec))

    engine = engine.resolve(strict=True)
    output = output.resolve()
    if output.exists():
        if output.is_symlink() or not output.is_dir():
            raise ValueError("closure output must be a real directory")
        shutil.rmtree(output)
    (output / "bin").mkdir(parents=True)
    (output / "lib").mkdir()
    (output / "licenses").mkdir()
    shutil.copyfile(engine, output / "bin/tesseract")
    shutil.copymode(engine, output / "bin/tesseract")

    def component_for(path: Path) -> dict:
        resolved = path.resolve(strict=True)
        matches = [spec for prefix, spec in roots if resolved == prefix or prefix in resolved.parents]
        if len(matches) != 1:
            raise ValueError(f"no unique pinned component owns {resolved.name}")
        return matches[0]

    queue = [engine]
    seen_sources: set[Path] = set()
    libraries: dict[str, dict] = {}
    while queue:
        owner = queue.pop(0).resolve(strict=True)
        if owner in seen_sources:
            continue
        seen_sources.add(owner)
        for dependency in _otool(owner):
            if dependency.startswith(("/usr/lib/", "/System/Library/")):
                continue
            source = _resolve_dependency(dependency, owner, engine)
            if not str(source).startswith("/opt/homebrew/"):
                raise ValueError(f"unsupported non-system dependency: {dependency}")
            logical = Path(dependency).name
            spec = component_for(source)
            digest = sha256(source)
            existing = libraries.get(logical)
            if existing and existing["sha256"] != digest:
                raise ValueError(f"dylib basename collision: {logical}")
            if not existing:
                destination = output / "lib" / logical
                shutil.copyfile(source, destination)
                shutil.copymode(source, destination)
                libraries[logical] = {
                    "filename": logical, "sha256": digest,
                    "size_bytes": destination.stat().st_size,
                    "component": spec["formula"], "version": spec["version"],
                    "license": spec["license"], "homepage": spec["homepage"],
                }
                queue.append(source)

    used_components = {"tesseract", *(entry["component"] for entry in libraries.values())}
    licenses: list[dict] = []
    for prefix, spec in roots:
        if spec["formula"] not in used_components:
            continue
        files = spec.get("license_files")
        if not isinstance(files, list) or not files:
            raise ValueError(f"component {spec['formula']} has no license files")
        for relative in files:
            source = (prefix / str(relative)).resolve(strict=True)
            if prefix not in source.parents or not source.is_file() or source.is_symlink():
                raise ValueError(f"invalid license file for {spec['formula']}: {relative}")
            filename = f"{spec['formula']}-{source.name}"
            destination = output / "licenses" / filename
            shutil.copyfile(source, destination)
            licenses.append({
                "path": f"licenses/{filename}", "component": spec["formula"],
                "version": spec["version"], "license": spec["license"],
                "sha256": sha256(destination), "size_bytes": destination.stat().st_size,
            })

    manifest = {
        "schema": "mpdf-macos-ocr-library-inputs", "schema_version": "1.0",
        "target": target,
        "engine": {"path": "bin/tesseract", "sha256": sha256(output / "bin/tesseract")},
        "libraries": sorted(libraries.values(), key=lambda item: item["filename"]),
        "licenses": sorted(licenses, key=lambda item: item["path"]),
    }
    path = output / "library-manifest.json"
    path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return path


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--components", type=Path, default=DEFAULT_COMPONENTS)
    args = parser.parse_args()
    try:
        print(collect(args.engine, args.output, args.components))
    except (OSError, ValueError, subprocess.CalledProcessError) as exc:
        parser.exit(1, f"closure collection failed: {exc}\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
