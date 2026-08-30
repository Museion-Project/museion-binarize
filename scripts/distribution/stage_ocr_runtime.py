#!/usr/bin/env python3
"""Stage a relocatable, auditable optional local OCR plugin artifact.

This is deliberately a *staging* tool, not a dependency installer.  Every
input is explicit and the default path is network-free.  A real macOS build
may pass a frozen sidecar, a Tesseract installation root and a provisioned
``tessdata_best`` directory.  Tests can pass ordinary executable fixtures;
the generated manifest records whether Mach-O inspection was performed.

The output directory is an independent artifact directory (normally under
``target`` or ``dist-out``), never a source-tree Git path. The base release
does not require this artifact. The application does not invoke this module at
runtime.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PINNED = ROOT / "distribution/ocr-models/manifest.toml"
MANIFEST_NAME = "runtime-manifest.json"
TARGET_ARCH = {"aarch64-apple-darwin": "arm64", "x86_64-apple-darwin": "x86_64"}
ALLOWED_SPDX = {"MIT", "Apache-2.0", "MIT OR Apache-2.0", "Apache-2.0 OR MIT"}


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def _real_file(path: Path, label: str) -> Path:
    path = path.expanduser()
    if path.is_symlink() or not path.is_file():
        raise ValueError(f"{label} must be a real file: {path}")
    return path.resolve()


def _load_model_set(path: Path, name: str) -> dict:
    data = tomllib.loads(path.read_text(encoding="utf-8"))
    if data.get("schema") != "mpdf-ocr-model-manifest":
        raise ValueError("unexpected OCR model manifest schema")
    matches = [item for item in data.get("model_set", []) if item.get("name") == name]
    if len(matches) != 1:
        raise ValueError(f"model set {name!r} is not uniquely pinned")
    selected = matches[0]
    if not selected.get("redistributable"):
        raise ValueError(f"model set {name!r} is not redistributable")
    return selected


def _copy(source: Path, destination: Path) -> dict:
    source = _real_file(source, "runtime input")
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, destination)
    shutil.copymode(source, destination)
    return {"sha256": sha256(destination), "size_bytes": destination.stat().st_size}


def _mach_o(path: Path) -> bool:
    try:
        return path.read_bytes()[:4] in {
            b"\xfe\xed\xfa\xce", b"\xce\xfa\xed\xfe", b"\xfe\xed\xfa\xcf",
            b"\xcf\xfa\xed\xfe", b"\xca\xfe\xba\xbe", b"\xbe\xba\xfe\xca",
        }
    except OSError:
        return False


def _arch(path: Path, expected: str, *, allow_test_fixture: bool = False) -> str:
    """Return an architecture observation; reject a known wrong Mach-O.

    ``file``/``lipo`` are intentionally optional so this remains testable on
    Linux and with byte fixtures.  A fixture may carry ``.arch`` sidecar
    metadata, which is useful for CI without pretending it is Mach-O.
    """
    marker = path.with_name(path.name + ".arch")
    observed = marker.read_text(encoding="utf-8").strip() if marker.is_file() else None
    if observed and observed != expected:
        raise ValueError(f"{path.name} architecture {observed!r} does not match {expected!r}")
    # The test-only API is intentionally allowed to use ordinary byte files,
    # including files beginning with Mach-O magic. It must never claim that
    # those bytes were inspected as a production binary.
    if allow_test_fixture:
        return observed or ("test-fixture" if _mach_o(path) else expected)
    if _mach_o(path) and not shutil.which("lipo"):
        if not allow_test_fixture:
            raise ValueError("lipo is required for production Mach-O inspection")
        return observed or "test-fixture"
    if _mach_o(path) and shutil.which("lipo"):
        result = subprocess.run(["lipo", "-archs", str(path)], capture_output=True, text=True)
        arches = result.stdout.split()
        if result.returncode or expected not in arches:
            raise ValueError(f"{path.name} does not contain required {expected} Mach-O slice")
        return "+".join(arches)
    return observed or (expected if not _mach_o(path) else "uninspected-macho")


def _otool_state(path: Path, *, allow_test_fixture: bool = False) -> tuple[list[str], list[str]]:
    if not _mach_o(path):
        return [], []
    if not shutil.which("otool"):
        if allow_test_fixture:
            return [], []
        raise ValueError("otool is required for production Mach-O inspection")
    deps_result = subprocess.run(["otool", "-L", str(path)], capture_output=True, text=True)
    if deps_result.returncode:
        raise ValueError(f"otool could not inspect {path.name}")
    deps = [line.strip().split(" ", 1)[0] for line in deps_result.stdout.splitlines()[1:] if line.strip()]
    load = subprocess.run(["otool", "-l", str(path)], capture_output=True, text=True, check=True).stdout
    rpaths: list[str] = []
    lines = load.splitlines()
    for index, line in enumerate(lines):
        if line.strip() == "cmd LC_RPATH":
            for candidate in lines[index + 1:index + 5]:
                if candidate.strip().startswith("path "):
                    rpaths.append(candidate.strip().split(" ", 2)[1])
                    break
    return deps, rpaths


def _otool_dependencies(path: Path, *, allow_test_fixture: bool = False) -> list[str]:
    return _otool_state(path, allow_test_fixture=allow_test_fixture)[0]


def _detect_version(engine: Path) -> str:
    try:
        result = subprocess.run([str(engine), "--version"], capture_output=True, text=True,
                                timeout=15, check=True)
    except (OSError, subprocess.SubprocessError) as exc:
        raise ValueError("Tesseract version is required (--tesseract-version) when it cannot be detected") from exc
    match = re.search(r"(?<!\d)(\d+\.\d+\.\d+)(?!\d)", result.stdout + result.stderr)
    if not match:
        raise ValueError("Tesseract --version did not report a semantic version")
    return match.group(1)


def _rewrite_macho(path: Path, staged_root: Path, staged_libs: dict[str, Path], *, allow_test_fixture: bool = False) -> tuple[list[str], list[str]]:
    """Rewrite non-system install names to loader-relative paths.

    System libraries remain system references.  On non-macOS or byte fixtures
    this is a no-op and the manifest records that Mach-O rewriting was not
    applicable.
    """
    deps, rpaths = _otool_state(path, allow_test_fixture=allow_test_fixture)
    rewritten: list[str] = []
    if not deps:
        return rewritten, rpaths
    if not shutil.which("install_name_tool"):
        raise ValueError(f"install_name_tool is required for Mach-O input {path.name}")
    for old in deps:
        # ``otool -L`` reports a dylib's own install name as its first entry;
        # it is not a dependency and is set explicitly below.
        if path.suffix == ".dylib" and old in {path.name, "@rpath/" + path.name}:
            continue
        if old.startswith(("/usr/lib/", "/System/Library/")):
            continue
        source_name = Path(old).name
        if source_name not in staged_libs:
            raise ValueError(f"missing non-system dylib closure for {path.name}: {old}")
        owner_dir = path.parent
        library = staged_libs[source_name]
        relative = os.path.relpath(library, owner_dir).replace(os.sep, "/")
        new = "@loader_path/" + relative
        subprocess.run(["install_name_tool", "-change", old, new, str(path)], check=True)
        # Do not copy a machine-local Homebrew path into provenance.  The
        # basename remains sufficient to audit the closure mapping while the
        # verifier can guarantee that forbidden absolute paths never appear in
        # a release manifest.
        old_observation = old
        if old.startswith(("/opt/homebrew", "/usr/local", "/Library/Frameworks")):
            old_observation = "<external>/" + source_name
        rewritten.append(f"{old_observation} -> {new}")
    for rpath in rpaths:
        if rpath.startswith(("/usr/lib/", "/System/Library/", "@")):
            continue
        subprocess.run(["install_name_tool", "-delete_rpath", rpath, str(path)], check=True)
        rewritten.append("rpath <external> -> removed")
    # Every non-system dependency above is rewritten directly to an
    # owner-relative @loader_path, so adding an LC_RPATH is unnecessary (and
    # can fail when that rpath already exists). Existing loader-relative
    # rpaths are harmless; external absolute rpaths were removed above.
    if path.suffix == ".dylib":
        subprocess.run(["install_name_tool", "-id", "@rpath/" + path.name, str(path)], check=True)
    return rewritten, rpaths


def _verify_rewritten(path: Path, staged_root: Path) -> None:
    deps, rpaths = _otool_state(path)
    for dependency in deps:
        # otool reports a dylib's own install-id as the first line. It is not
        # a load dependency and is deliberately kept relocatable as @rpath.
        if path.suffix == ".dylib" and dependency == "@rpath/" + path.name:
            continue
        if dependency.startswith(("/usr/lib/", "/System/Library/")):
            continue
        if dependency.startswith("@rpath/"):
            raise ValueError(f"unresolved @rpath dependency after rewrite: {path.name}: {dependency}")
        if dependency.startswith("@loader_path/"):
            resolved = (path.parent / dependency.removeprefix("@loader_path/")).resolve()
            if staged_root not in resolved.parents or not resolved.is_file():
                raise ValueError(f"loader-relative dependency escapes closure: {path.name}: {dependency}")
            continue
        if dependency.startswith("/"):
            raise ValueError(f"external absolute dependency after rewrite: {path.name}: {dependency}")
        raise ValueError(f"unrecognized Mach-O dependency after rewrite: {path.name}: {dependency}")
    for rpath in rpaths:
        if rpath.startswith(("@", "/usr/lib/", "/System/Library/")):
            continue
        raise ValueError(f"external LC_RPATH after rewrite: {path.name}: {rpath}")


def stage(*, target: str, sidecar: Path, tesseract_root: Path, models_dir: Path,
          out_dir: Path, release: str = "0.1.0-rc.3", model_set: str = "tessdata_best",
          tesseract_binary: Path | None = None, tesseract_version: str | None = None,
          sidecar_version: str = "mpdf-ocr/0.1",
          sidecar_license: Path | None = None, tesseract_license: Path | None = None,
          sidecar_license_expression: str | None = None,
          tessdata_license: Path | None = None, pinned_models: Path = PINNED,
          dependency_manifest: Path | None = None, library_manifest: Path | None = None,
          allow_test_fixture: bool = False) -> Path:
    if target not in TARGET_ARCH:
        raise ValueError(f"unsupported OCR runtime target: {target}")
    if sidecar_license_expression not in ALLOWED_SPDX:
        raise ValueError("sidecar license expression must be one of the allowed SPDX expressions")
    sidecar = _real_file(sidecar, "frozen sidecar")
    tesseract_root = tesseract_root.expanduser().resolve(strict=True)
    models_dir = models_dir.expanduser().resolve(strict=True)
    if not tesseract_root.is_dir() or not models_dir.is_dir():
        raise ValueError("Tesseract root and model directory must be directories")
    engine = _real_file(tesseract_binary or (tesseract_root / "bin/tesseract"), "Tesseract binary")
    selected = _load_model_set(pinned_models, model_set)
    expected_arch = TARGET_ARCH[target]
    tesseract_version = tesseract_version or _detect_version(engine)
    runtime = out_dir / "ocr-runtime" if out_dir.name != "ocr-runtime" else out_dir
    if runtime.exists():
        if runtime.is_symlink() or not runtime.is_dir():
            raise ValueError(f"output runtime is not a directory: {runtime}")
        shutil.rmtree(runtime)
    runtime.mkdir(parents=True)
    runtime = runtime.resolve(strict=True)
    entries: list[dict] = []
    sources: list[dict] = []

    def add(source: Path, relative: str, role: str, **extra: object) -> Path:
        destination = runtime / relative
        record = _copy(source, destination)
        entry = {"path": relative, "role": role, **record, **extra}
        entries.append(entry)
        return destination

    staged_engine = add(engine, "bin/tesseract", "engine", architecture=_arch(engine, expected_arch, allow_test_fixture=allow_test_fixture),
                        version=tesseract_version, license="Apache-2.0")
    staged_sidecar = add(sidecar, "bin/mpdf-ocr-sidecar", "sidecar", architecture=_arch(sidecar, expected_arch, allow_test_fixture=allow_test_fixture),
                         version=sidecar_version, license=sidecar_license_expression)
    for spec in selected.get("file", []):
        source = models_dir / spec["filename"]
        source = _real_file(source, f"model {spec['language']}")
        if source.stat().st_size != spec["size_bytes"] or sha256(source) != spec["sha256"]:
            raise ValueError(f"model {spec['filename']} does not match pinned bytes")
        add(source, f"tessdata/{spec['filename']}", "model", language=spec["language"],
            model_version=selected["version"], license=selected["license"])

    # A sidecar-facing manifest is an auditable copy of the pinned model
    # records.  It is data, not an untracked dependency discovered at runtime.
    model_manifest = runtime / "tessdata/manifest.json"
    model_manifest.write_text(json.dumps({
        "schema": "mpdf-ocr-models", "schema_version": "1.0", "model_set": model_set,
        "model_set_version": selected["version"], "engine": "tesseract",
        "license": selected["license"], "license_url": selected["license_url"],
        "models": [{"language": s["language"], "filename": s["filename"],
                     "engine": "tesseract", "source": s["url"],
                     "sha256": s["sha256"], "size_bytes": s["size_bytes"],
                     "model_version": selected["version"], "license": selected["license"]}
                    for s in selected["file"]],
    }, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    model_manifest.chmod(0o644)
    entries.append({"path": "tessdata/manifest.json", "role": "model-manifest",
                    "sha256": sha256(model_manifest), "size_bytes": model_manifest.stat().st_size,
                    "license": selected["license"]})

    licenses = (("license-tesseract", tesseract_license), ("license-tessdata", tessdata_license),
                ("license-sidecar", sidecar_license))
    for role, source in licenses:
        if source is None:
            raise ValueError(f"explicit license input required for {role}")
        destination = f"licenses/{role.removeprefix('license-').upper()}.txt"
        add(source, destination, role, license="Apache-2.0" if role != "license-sidecar" else sidecar_license_expression)

    # Copy the complete non-system closure explicitly supplied under the root.
    # On macOS, otool inspection additionally proves that every non-system
    # install name is represented and gets rewritten to @loader_path.
    library_records: dict[str, dict] = {}
    library_license_records: list[dict] = []
    if library_manifest is not None:
        library_data = json.loads(library_manifest.read_text(encoding="utf-8"))
        if (library_data.get("schema") != "mpdf-macos-ocr-library-inputs"
                or library_data.get("schema_version") != "1.0"
                or library_data.get("target") != target):
            raise ValueError("library input manifest identity mismatch")
        raw_libraries = library_data.get("libraries", [])
        library_records = {item["filename"]: item for item in raw_libraries}
        library_license_records = library_data.get("licenses", [])
        if len(library_records) != len(raw_libraries):
            raise ValueError("duplicate library input filenames")
    elif not allow_test_fixture:
        raise ValueError("production staging requires a per-library component manifest")

    staged_libs: dict[str, Path] = {}
    library_sources: list[Path] = []
    seen_library_sources: set[Path] = set()
    for base in (tesseract_root, sidecar.parent):
        for source in sorted(base.rglob("*.dylib")):
            if source.is_symlink() or not source.is_file():
                continue
            resolved_source = source.resolve()
            if resolved_source == engine or resolved_source in seen_library_sources:
                continue
            seen_library_sources.add(resolved_source)
            library_sources.append(source)
    for source in sorted(library_sources):
        if source.is_symlink() or not source.is_file():
            continue
        name = source.name
        record = library_records.get(name)
        if record is None and not allow_test_fixture:
            raise ValueError(f"library {name} is absent from the component manifest")
        if record is not None and (record.get("sha256") != sha256(source)
                                   or record.get("size_bytes") != source.stat().st_size
                                   or not record.get("license") or not record.get("component")
                                   or not record.get("version")):
            raise ValueError(f"library component metadata mismatch: {name}")
        staged = add(
            source, f"lib/{name}", "runtime-library",
            architecture=_arch(source, expected_arch, allow_test_fixture=allow_test_fixture),
            license=record.get("license", "Apache-2.0") if record else "Apache-2.0",
            component=record.get("component", "test-fixture") if record else "test-fixture",
            component_version=record.get("version", "test-fixture") if record else "test-fixture",
            source_url=record.get("homepage", "NOASSERTION") if record else "NOASSERTION",
        )
        staged_libs[name] = staged
    if not allow_test_fixture and set(staged_libs) != set(library_records):
        raise ValueError("library component manifest does not exactly match the collected closure")
    for record in library_license_records:
        relative = record.get("path")
        if not isinstance(relative, str) or not relative.startswith("licenses/"):
            raise ValueError("invalid runtime library license path")
        source = tesseract_root / relative
        if (not source.is_file() or source.is_symlink()
                or sha256(source) != record.get("sha256")
                or source.stat().st_size != record.get("size_bytes")):
            raise ValueError(f"runtime library license mismatch: {relative}")
        add(
            source, f"licenses/runtime-{Path(relative).name}", "license-runtime",
            license=record.get("license"), component=record.get("component"),
            component_version=record.get("version"),
        )
    rewrite_log = []
    rpath_observations = []
    dependency_observations: list[dict[str, str]] = []
    post_dependency_observations: list[dict[str, str]] = []
    post_rpath_observations: list[dict[str, str]] = []
    for path in (staged_engine, staged_sidecar, *staged_libs.values()):
        for dependency in _otool_dependencies(path, allow_test_fixture=allow_test_fixture):
            dependency_observations.append({"owner": path.name, "path": dependency})
        rewritten, rpaths = _rewrite_macho(path, runtime, staged_libs,
                                            allow_test_fixture=allow_test_fixture)
        rewrite_log.extend(rewritten)
        rpath_observations.extend({"owner": path.name, "path": rpath} for rpath in rpaths)
    all_binaries = (engine, sidecar, *library_sources)
    inspection_mode = "mach-o" if not allow_test_fixture and (
        all(_mach_o(path) for path in all_binaries)
        and all(shutil.which(tool) for tool in ("lipo", "otool", "install_name_tool"))
    ) else "test-fixture"
    if not allow_test_fixture and inspection_mode != "mach-o":
        raise ValueError("macOS runtime staging requires Mach-O engine, sidecar, and dylib inputs")
    post_rewrite_verified = False
    if inspection_mode == "mach-o":
        for path in (staged_engine, staged_sidecar, *staged_libs.values()):
            _verify_rewritten(path, runtime)
            dependencies, rpaths = _otool_state(path)
            post_dependency_observations.extend(
                {"owner": path.name, "path": dependency} for dependency in dependencies
            )
            post_rpath_observations.extend(
                {"owner": path.name, "path": rpath} for rpath in rpaths
            )
        if shutil.which("codesign") is None:
            raise ValueError("codesign is required to re-sign rewritten arm64 Mach-O files")
        # install_name_tool invalidates existing signatures. Sign libraries
        # first, then executables, so the standalone runtime can execute on
        # Apple Silicon before the enclosing app receives its final identity.
        for path in (*staged_libs.values(), staged_engine, staged_sidecar):
            subprocess.run(
                ["codesign", "--force", "--sign", "-", "--timestamp=none", str(path)],
                check=True, capture_output=True,
            )
            subprocess.run(["codesign", "--verify", "--strict", str(path)], check=True,
                           capture_output=True)
        post_rewrite_verified = True

    # Ensure no accidental absolute/Homebrew reference is carried as source
    # metadata.  The verifier independently applies the same policy.
    software = []
    if dependency_manifest is not None:
        try:
            dependency_data = json.loads(dependency_manifest.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as exc:
            raise ValueError(f"cannot read dependency manifest: {exc}") from exc
        software = dependency_data.get("software", dependency_data) if isinstance(dependency_data, dict) else dependency_data
        if not isinstance(software, list):
            raise ValueError("dependency manifest software must be a list")
        for item in software:
            if not isinstance(item, dict) or not isinstance(item.get("name"), str) or not isinstance(item.get("version"), str):
                raise ValueError("dependency manifest has an invalid software record")
            if not re.fullmatch(r"[0-9a-fA-F]{64}", str(item.get("sha256", ""))):
                raise ValueError(f"dependency {item.get('name', '<unknown>')} has no SHA-256")
            if not isinstance(item.get("license"), str) or not item["license"]:
                raise ValueError(f"dependency {item['name']} has no license")
    source_records = [
        {"role": "engine", "name": engine.name, "sha256": sha256(engine),
         "version": tesseract_version, "license": "Apache-2.0"},
        {"role": "sidecar", "name": sidecar.name, "sha256": sha256(sidecar),
         "version": sidecar_version, "license": sidecar_license_expression},
        {"role": "model-set", "name": model_set, "version": selected["version"],
         "license": selected["license"], "source": selected["upstream_url"],
         "files": [{"language": spec["language"], "sha256": spec["sha256"],
                    "size_bytes": spec["size_bytes"]} for spec in selected["file"]]},
    ]
    source_records.extend(
        {"role": "runtime-library", "name": source.name, "sha256": sha256(source),
         "version": library_records.get(source.name, {}).get("version", "test-fixture"),
         "component": library_records.get(source.name, {}).get("component", "test-fixture"),
         "license": library_records.get(source.name, {}).get("license", "Apache-2.0")}
        for source in library_sources
    )
    provenance = {
        "tool": "stage_ocr_runtime.py",
        "target": target,
        "architecture": expected_arch,
        "source_policy": "explicit-inputs-only; network-disabled",
        "sources": source_records,
        "install_name_rewrites": rewrite_log,
        # These are deliberately post-rewrite observations.  Source install
        # names are not copied into the artifact, and therefore cannot leak a
        # developer's absolute Homebrew path into release provenance.
        "rpaths": post_rpath_observations,
        "inspection_mode": inspection_mode,
        "macho_required": any(_mach_o(path) for path in (staged_engine, staged_sidecar, *staged_libs.values())),
        "post_rewrite_verified": post_rewrite_verified,
        "dependencies": post_dependency_observations,
        "software": software,
        "dependency_policy": {"system_paths_allowed": ["/usr/lib", "/System/Library"],
                               "forbidden_prefixes": ["/opt/homebrew", "/usr/local", "/Library/Frameworks"]},
    }
    manifest = {
        "schema": "mpdf-ocr-runtime-bundle", "schema_version": "1.0", "release": release,
        "target": target, "architecture": expected_arch, "engine": "tesseract",
        "inspection_mode": inspection_mode,
        "post_rewrite_verified": post_rewrite_verified,
        "ad_hoc_signed": inspection_mode == "mach-o",
        "engine_version": tesseract_version, "engine_license": "Apache-2.0",
        "sidecar_protocol": "mpdf-ocr/0.1", "sidecar_version": sidecar_version,
        "model_set": model_set, "model_set_version": selected["version"],
        "requires_system_python": False, "requires_system_tesseract": False,
        "provenance": provenance, "files": sorted(entries, key=lambda x: x["path"]),
    }
    # Mach-O rewriting and ad-hoc signing change bytes after the initial copy.
    # Refresh every staged file identity immediately before serializing the
    # authoritative inventory.
    for entry in manifest["files"]:
        staged = runtime.joinpath(*Path(entry["path"]).parts)
        entry["sha256"] = sha256(staged)
        entry["size_bytes"] = staged.stat().st_size
    manifest_path = runtime / MANIFEST_NAME
    manifest_path.write_text(json.dumps(manifest, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return runtime


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True)
    parser.add_argument("--sidecar", type=Path, required=True)
    parser.add_argument("--tesseract-root", type=Path, required=True)
    parser.add_argument("--tesseract-binary", type=Path)
    parser.add_argument("--tesseract-version")
    parser.add_argument("--sidecar-version", default="mpdf-ocr/0.1")
    parser.add_argument("--models-dir", type=Path, required=True)
    parser.add_argument("--out-dir", type=Path, required=True)
    parser.add_argument("--release", default="0.1.0-rc.3")
    parser.add_argument("--model-set", default="tessdata_best")
    parser.add_argument("--pinned-models", type=Path, default=PINNED)
    parser.add_argument("--sidecar-license", type=Path, required=True)
    parser.add_argument("--sidecar-license-expression", required=True)
    parser.add_argument("--tesseract-license", type=Path, required=True)
    parser.add_argument("--tessdata-license", type=Path, required=True)
    parser.add_argument("--dependency-manifest", type=Path,
                        help="optional frozen sidecar software inventory JSON (e.g. Pillow)")
    parser.add_argument("--library-manifest", type=Path,
                        help="required production dylib component/SHA/license inventory")
    args = parser.parse_args()
    try:
        runtime = stage(**vars(args))
        print(runtime)
    except (OSError, ValueError, subprocess.CalledProcessError) as exc:
        print(f"OCR runtime staging failed: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
