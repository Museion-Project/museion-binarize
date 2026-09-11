#!/usr/bin/env python3
"""Generate a deterministic SPDX 2.3 JSON SBOM for a release target.

The generator is deliberately network-free: Cargo metadata and the checked-in
lockfiles are inputs, while package downloads are the caller's responsibility.
Tests can inject metadata/lock text directly. No absolute paths, usernames,
hostnames, or user-provisioned OCR models are emitted; the only timestamp is
the explicit reproducible SPDX creation time.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import tomllib
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[2]


def _checksum(value: str) -> str:
    return hashlib.sha256(value.encode()).hexdigest()


def _license_for(package: dict[str, Any]) -> str:
    value = package.get("license") or package.get("license_file")
    if isinstance(value, str) and value and not re.match(r"^(?:[/\\]|[A-Za-z]:[/\\]|file:)", value):
        return value
    return "NOASSERTION"


def _download_location(package: dict[str, Any]) -> str:
    value = package.get("repository")
    if not isinstance(value, str) or re.match(r"^(?:[/\\]|[A-Za-z]:[/\\]|file:)", value):
        return "NOASSERTION"
    return value


def _relative_manifest(path: str, metadata: dict[str, Any]) -> str:
    """Return a repository-stable manifest path (never an absolute path)."""
    raw = str(path).replace("\\", "/")
    root = str(metadata.get("workspace_root", "")).replace("\\", "/").rstrip("/")
    if root and raw.startswith(root + "/"):
        return raw[len(root) + 1 :]
    # Cargo registry/git paths are not part of the release source tree.  Keep
    # only the stable package identity for those packages; this also makes
    # metadata from two different checkouts byte-for-byte equivalent.
    for marker in ("/crates/", "/apps/", "/examples/"):
        if marker in raw:
            return marker.strip("/").split("/", 1)[0] + "/" + raw.split(marker, 1)[1]
    return Path(raw).name if raw else "unknown/Cargo.toml"


def _stable_package_key(package: dict[str, Any], metadata: dict[str, Any]) -> str:
    name = str(package.get("name", ""))
    version = str(package.get("version", ""))
    source = str(package.get("source") or "")
    # A Cargo package id contains an absolute path for path dependencies in
    # some Cargo versions.  Source URLs and the repo-relative manifest path
    # are the portable identity instead.
    if source.startswith("registry+"):
        source = "registry:" + source[len("registry+") :].split("#", 1)[0]
    elif source.startswith("git+"):
        # Keep the repository and revision: two git revisions are distinct
        # resolved dependencies even when their manifests share a version.
        source = "git:" + source[len("git+") :]
    elif source.startswith("path+"):
        source = "workspace"
    else:
        source = "workspace" if not source else re.sub(r"[A-Za-z]:|/[^#]+", "", source)
    manifest = _relative_manifest(str(package.get("manifest_path", "")), metadata)
    return f"{source}|{name}|{version}|{manifest}"


def _rust_spdx_id(package: dict[str, Any], metadata: dict[str, Any]) -> str:
    key = _stable_package_key(package, metadata)
    safe = re.sub(r"[^A-Za-z0-9.-]", "-", f"{package.get('name', '')}-{package.get('version', '')}")
    return f"SPDXRef-Rust-{safe}-{_checksum(key)[:12]}"


def _rust_components(metadata: dict[str, Any]) -> list[dict[str, Any]]:
    packages = {p["id"]: p for p in metadata.get("packages", []) if p.get("id")}
    components = []
    for package in packages.values():
        name = str(package.get("name", ""))
        version = str(package.get("version", ""))
        if not name or not version:
            continue
        components.append(
            {
                # The same crate name/version can occur from crates.io and a
                # git/path source. Include a stable id fragment to keep SPDX
                # identifiers unique without leaking the local checkout.
                "SPDXID": _rust_spdx_id(package, metadata),
                "name": name,
                "versionInfo": version,
                "downloadLocation": _download_location(package),
                "licenseConcluded": _license_for(package),
                "licenseDeclared": _license_for(package),
                "filesAnalyzed": False,
                "externalRefs": [
                    {
                        "referenceCategory": "PACKAGE-MANAGER",
                        "referenceType": "purl",
                        "referenceLocator": f"pkg:cargo/{name}@{version}",
                    }
                ],
            }
        )
    return components


def _rust_relationships(metadata: dict[str, Any]) -> list[dict[str, str]]:
    """Preserve Cargo's resolved graph edges without embedding local paths."""
    ids = {
        package.get("id"): _rust_spdx_id(package, metadata)
        for package in metadata.get("packages", [])
        if package.get("id")
    }
    relationships: list[dict[str, str]] = []
    for node in metadata.get("resolve", {}).get("nodes", []):
        source = ids.get(node.get("id"))
        if not source:
            continue
        for dependency in node.get("dependencies", []):
            target_id = dependency.get("pkg") if isinstance(dependency, dict) else dependency
            target = ids.get(target_id)
            if target:
                relationships.append({"spdxElementId": source, "relationshipType": "DEPENDS_ON", "relatedSpdxElement": target})
    return sorted(relationships, key=lambda r: (r["spdxElementId"], r["relatedSpdxElement"]))


def _node_components(lock_text: str | None, node_modules: Path | None,
                     node_graph: list[dict[str, Any]] | None = None) -> tuple[list[dict[str, Any]], list[dict[str, str]]]:
    records: dict[tuple[str, str], dict[str, Any]] = {}
    # pnpm lock v6/v9 package snapshots use /name@version (or name@version).
    # Once an installed graph or node_modules exists, do not append lock-only
    # packages that are not part of the resolved desktop installation.
    installed_source = bool(node_graph) or bool(node_modules and node_modules.is_dir())
    if lock_text and not installed_source:
        for match in re.finditer(r"(?:^|\n)\s{2,}(?:/)?(@?[^\s:/]+(?:/[^\s:/]+)?)@([0-9][^\s(:]+):", lock_text):
            records.setdefault((match.group(1), match.group(2)), {"name": match.group(1), "version": match.group(2)})
    allowed = {(item.get("name"), item.get("version")) for item in (node_graph or [])
               if isinstance(item, dict)}
    if node_modules and node_modules.is_dir():
        package_paths = list(node_modules.glob("**/package.json"))
        store = node_modules / ".pnpm"
        if store.is_dir():
            package_paths.extend(store.rglob("package.json"))
        for package_json in sorted(set(package_paths)):
            try:
                data = json.loads(package_json.read_text())
            except (OSError, json.JSONDecodeError):
                continue
            if isinstance(data.get("name"), str) and isinstance(data.get("version"), str):
                if node_graph and (data["name"], data["version"]) not in allowed:
                    continue
                item = {"name": data["name"], "version": data["version"],
                        "license": data.get("license"), "repository": data.get("repository"),
                        "dependencies": data.get("dependencies", {})}
                records[(data["name"], data["version"])] = item
    for item in node_graph or []:
        if isinstance(item, dict) and isinstance(item.get("name"), str) and isinstance(item.get("version"), str):
            records[(item["name"], item["version"])] = dict(item)
    result = []
    ids: dict[tuple[str, str], str] = {}
    for name, version in sorted(records):
        safe = re.sub(r"[^A-Za-z0-9.-]", "-", f"{name}-{version}")
        identifier = f"SPDXRef-Node-{safe}-{_checksum(f'{name}|{version}')[:12]}"
        # A valid pnpm graph cannot contain duplicate name/version nodes.
        ids[(name, version)] = identifier
        item = records[(name, version)]
        license_value = item.get("license")
        if isinstance(license_value, dict):
            license_value = license_value.get("type") or license_value.get("name")
        if not isinstance(license_value, str) or re.match(r"^(?:[/\\]|[A-Za-z]:[/\\]|file:)", license_value):
            license_value = "NOASSERTION"
        repository = item.get("repository")
        if isinstance(repository, dict):
            repository = repository.get("url")
        location = item.get("downloadLocation") or repository or "NOASSERTION"
        if not isinstance(location, str) or re.match(r"^(?:[/\\]|[A-Za-z]:[/\\]|file:)", location):
            location = "NOASSERTION"
        result.append(
            {
                "SPDXID": identifier,
                "name": name,
                "versionInfo": version,
                "downloadLocation": location,
                "licenseConcluded": str(license_value or "NOASSERTION"),
                "licenseDeclared": str(license_value or "NOASSERTION"),
                "filesAnalyzed": False,
                "externalRefs": [
                    {
                        "referenceCategory": "PACKAGE-MANAGER",
                        "referenceType": "purl",
                        "referenceLocator": f"pkg:npm/{name}@{version}",
                    }
                ],
            }
        )
    relationships = []
    for key, item in records.items():
        source = ids[key]
        dependencies = item.get("dependencies", {})
        if isinstance(dependencies, list):
            dependencies = {name: "" for name in dependencies}
        if not isinstance(dependencies, dict):
            continue
        for name in sorted(dependencies):
            requested = dependencies[name]
            exact = (name, requested) if isinstance(requested, str) else None
            if exact in ids:
                target_id = ids[exact]
            else:
                candidates = sorted((k for k in ids if k[0] == name), key=lambda k: k[1])
                if not candidates:
                    continue
                target_id = ids[candidates[-1]]
            relationships.append({"spdxElementId": source, "relationshipType": "DEPENDS_ON", "relatedSpdxElement": target_id})
    return result, relationships


def _flatten_pnpm_graph(value: Any, records: dict[tuple[str, str], dict[str, Any]],
                        *, hinted_name: str | None = None) -> None:
    """Flatten ``pnpm list --json --depth Infinity`` without retaining paths.

    pnpm has changed the shape of dependency nodes slightly between major
    versions (a child may omit ``name`` and use its map key instead).  Only
    package identity, metadata, and resolved dependency edges are retained;
    local fields such as ``path`` are never copied to the SBOM; a resolved
    package URL may be retained as its download source.
    """
    if not isinstance(value, dict):
        return
    name = value.get("name") or hinted_name
    version = value.get("version")
    if not isinstance(name, str) or not isinstance(version, str):
        return
    dependencies: dict[str, Any] = {}
    for section in ("dependencies", "devDependencies", "optionalDependencies"):
        children = value.get(section)
        if not isinstance(children, dict):
            continue
        for child_name, child in children.items():
            if isinstance(child, dict):
                child_version = child.get("version")
                dependencies[child_name] = child_version if isinstance(child_version, str) else ""
                _flatten_pnpm_graph(child, records, hinted_name=child_name)
            elif isinstance(child, str):
                dependencies[child_name] = child
    item = {
        "name": name,
        "version": version,
        "license": value.get("license"),
        "repository": value.get("repository"),
        "downloadLocation": value.get("resolved") or value.get("downloadLocation"),
        "dependencies": dependencies,
    }
    previous = records.get((name, version))
    if previous is not None:
        # pnpm list generally omits license/repository fields. Preserve those
        # values when the package.json scan already supplied them while still
        # preferring the resolved graph's exact dependency edges.
        for key in ("license", "repository", "downloadLocation"):
            if item.get(key) is None and previous.get(key) is not None:
                item[key] = previous[key]
    records[(name, version)] = item


def load_node_graph(desktop_root: Path) -> list[dict[str, Any]]:
    """Read the actual installed desktop graph, without network access.

    ``pnpm list`` only inspects the local installation. A missing executable
    or absent install is tolerated so source-only checks can still generate a
    lockfile-based SBOM; CI package jobs install dependencies first, so their
    SBOMs contain the resolved graph.
    """
    try:
        result = subprocess.run(
            ["pnpm", "list", "--json", "--depth", "Infinity"],
            cwd=desktop_root, capture_output=True, text=True, check=True,
        )
        payload = json.loads(result.stdout)
    except (OSError, subprocess.CalledProcessError, json.JSONDecodeError):
        return []
    records: dict[tuple[str, str], dict[str, Any]] = {}
    roots = payload if isinstance(payload, list) else [payload]
    for root in roots:
        _flatten_pnpm_graph(root, records)
    # The list command's compact output can omit transitive nodes that are
    # deduped by pnpm. Walk package manifests from the listed desktop root to
    # recover the complete installed runtime/dev/optional graph, while never
    # admitting an unreachable package from the store index.
    package_index: dict[tuple[str, str], Path] = {}
    workspace_root = desktop_root.parent.parent
    store_root = workspace_root / "node_modules" / ".pnpm"
    for package_json in store_root.rglob("package.json") if store_root.is_dir() else ():
        try:
            data = json.loads(package_json.read_text())
        except (OSError, json.JSONDecodeError):
            continue
        if isinstance(data.get("name"), str) and isinstance(data.get("version"), str):
            package_index[(data["name"], data["version"])] = package_json
    root_json = desktop_root / "package.json"
    try:
        root_data = json.loads(root_json.read_text())
    except (OSError, json.JSONDecodeError):
        root_data = None
    if not isinstance(root_data, dict):
        return list(records.values())
    visited: set[tuple[str, str]] = set()
    manifest_records: dict[tuple[str, str], dict[str, Any]] = {}

    def resolve(base: Path, name: str, requested: Any) -> Path | None:
        for ancestor in (base, *base.parents):
            candidate = ancestor / "node_modules" / name / "package.json"
            if candidate.is_file():
                return candidate.resolve()
        versions = sorted(path for (pkg_name, version), path in package_index.items()
                          if pkg_name == name and (not isinstance(requested, str) or requested == version))
        return versions[-1].resolve() if versions else None

    def walk(data: dict[str, Any], path: Path, *, is_root: bool = False) -> None:
        name, version = data.get("name"), data.get("version")
        if not isinstance(name, str) or not isinstance(version, str):
            return
        key = (name, version)
        if key in visited:
            return
        visited.add(key)
        deps: dict[str, Any] = {}
        for section in ("dependencies", "devDependencies", "optionalDependencies"):
            section_data = data.get(section)
            if not isinstance(section_data, dict):
                continue
            for dep_name, requested in section_data.items():
                child = resolve(path.parent, dep_name, requested)
                if child is None:
                    # A dependency absent from the installation is not part
                    # of the installed graph; optional omissions are normal.
                    continue
                try:
                    child_data = json.loads(child.read_text())
                except (OSError, json.JSONDecodeError):
                    continue
                child_version = child_data.get("version")
                if not isinstance(child_version, str):
                    continue
                deps[dep_name] = child_version
                walk(child_data, child)
        manifest_records[key] = {
            "name": name, "version": version, "license": data.get("license"),
            "repository": data.get("repository"), "dependencies": deps,
        }

    walk(root_data, root_json, is_root=True)
    # Manifest traversal is authoritative for reachability; retain only its
    # nodes and merge any package metadata found in pnpm's compact graph.
    for key, item in manifest_records.items():
        compact = records.get(key, {})
        for field in ("license", "repository", "downloadLocation"):
            if item.get(field) is None and compact.get(field) is not None:
                item[field] = compact[field]
    return list(manifest_records.values())


def validate_installed_node_graph(node_graph: list[dict[str, Any]],
                                  components: list[dict[str, Any]],
                                  relationships: list[dict[str, str]]) -> None:
    """Fail closed when a production install yields only a partial graph."""
    if not node_graph:
        raise ValueError("pnpm installed graph is empty; refusing incomplete production SBOM")
    node_ids = {p["SPDXID"] for p in components if p["SPDXID"].startswith("SPDXRef-Node-")}
    node_edges = [r for r in relationships if r["spdxElementId"] in node_ids
                  and r["relationshipType"] == "DEPENDS_ON"]
    if not node_edges:
        raise ValueError("installed Node graph has no dependency relationships")
    with_metadata = sum(
        p.get("licenseDeclared") != "NOASSERTION" or p.get("downloadLocation") != "NOASSERTION"
        for p in components if p["SPDXID"].startswith("SPDXRef-Node-")
    )
    if not node_ids or with_metadata * 2 < len(node_ids):
        raise ValueError("installed Node graph metadata coverage is too low")


def _pdfium_component(pdfium_manifest: dict[str, Any], target: str) -> dict[str, Any]:
    assets = [a for a in pdfium_manifest.get("asset", []) if a.get("target_triple") == target]
    if len(assets) != 1:
        raise ValueError(f"expected one pinned PDFium asset for {target}, found {len(assets)}")
    asset = assets[0]
    digest = asset.get("library_sha256")
    if not isinstance(digest, str) or not re.fullmatch(r"[0-9a-fA-F]{64}", digest):
        raise ValueError("PDFium manifest has no valid library_sha256")
    return {
        "SPDXID": "SPDXRef-PDFium-" + re.sub(r"[^A-Za-z0-9.-]", "-", target),
        "name": "PDFium",
        "versionInfo": str(asset.get("version", "NOASSERTION")),
        "downloadLocation": str(asset.get("archive_url", "NOASSERTION")),
        "licenseConcluded": str(asset.get("pdfium_license", "BSD-3-Clause AND Apache-2.0")),
        "licenseDeclared": str(asset.get("pdfium_license", "BSD-3-Clause AND Apache-2.0")),
        "filesAnalyzed": False,
        "supplier": "Organization: PDFium / pdfium-binaries",
        "checksums": [{"algorithm": "SHA256", "checksumValue": digest.lower()}],
        "comment": f"pinned build {asset.get('build', 'NOASSERTION')}; target {target}",
    }


def _ocr_runtime_components(runtime_manifest: dict[str, Any], target: str) -> list[dict[str, Any]]:
    """Represent every staged OCR input in SPDX without leaking local paths."""
    if runtime_manifest.get("schema") != "mpdf-ocr-runtime-bundle":
        raise ValueError("OCR runtime manifest has an unexpected schema")
    if runtime_manifest.get("target") != target:
        raise ValueError("OCR runtime manifest target does not match SBOM target")
    files = runtime_manifest.get("files")
    if not isinstance(files, list) or not files:
        raise ValueError("OCR runtime manifest has no files")
    components: list[dict[str, Any]] = []
    for entry in files:
        if not isinstance(entry, dict) or not isinstance(entry.get("path"), str):
            raise ValueError("OCR runtime manifest contains an invalid file entry")
        digest = entry.get("sha256")
        if not isinstance(digest, str) or not re.fullmatch(r"[0-9a-fA-F]{64}", digest):
            raise ValueError("OCR runtime manifest contains an invalid SHA-256")
        role = str(entry.get("role", "runtime-library"))
        language = entry.get("language")
        name = {
            "engine": "Tesseract",
            "sidecar": "M PDF frozen OCR sidecar",
            "model": f"tessdata_best model ({language})" if language else "tessdata_best model",
            "license-tesseract": "Tesseract license",
            "license-tessdata": "tessdata_best license",
            "license-sidecar": "M PDF sidecar license",
            "license-runtime": f"OCR runtime license ({entry.get('component', Path(entry['path']).name)})",
            "model-manifest": "tessdata_best model manifest",
        }.get(role, str(entry.get("component") or f"OCR runtime library ({Path(entry['path']).name})"))
        version = (runtime_manifest.get("engine_version") if role == "engine" else
                   runtime_manifest.get("model_set_version") if role == "model" else
                   entry.get("component_version") if role in {"runtime-library", "license-runtime"} else
                   runtime_manifest.get("sidecar_version", "NOASSERTION"))
        provenance_sources = runtime_manifest.get("provenance", {}).get("sources", [])
        model_license = next((item.get("license") for item in provenance_sources
                              if isinstance(item, dict) and item.get("role") == "model-set"), None)
        license_value = (entry.get("license") or
                         (runtime_manifest.get("engine_license", "NOASSERTION") if role == "engine" else
                          model_license or runtime_manifest.get("license", "NOASSERTION")))
        identifier_key = f"ocr|{target}|{entry['path']}|{digest}"
        components.append({
            "SPDXID": "SPDXRef-OCR-" + _checksum(identifier_key)[:16],
            "name": name,
            "versionInfo": str(version or "NOASSERTION"),
            "downloadLocation": str(entry.get("source_url") or "NOASSERTION"),
            "licenseConcluded": str(license_value),
            "licenseDeclared": str(license_value),
            "filesAnalyzed": False,
            "checksums": [{"algorithm": "SHA256", "checksumValue": digest.lower()}],
            "comment": f"role={role}; staged path={entry['path']}",
        })
    # SPDX IDs are content/path based, but duplicate entries still indicate a
    # malformed manifest and should never be emitted ambiguously.
    if len({item["SPDXID"] for item in components}) != len(components):
        raise ValueError("OCR runtime manifest contains duplicate files")
    # A frozen Python sidecar may carry a lock-generated software inventory
    # (Pillow and its image/runtime dependencies do not necessarily appear as
    # files in the bundle manifest).  Include those records when supplied,
    # while retaining the same no-local-path rule as the main SBOM.
    software = runtime_manifest.get("provenance", {}).get("software", [])
    if software:
        if not isinstance(software, list):
            raise ValueError("OCR provenance software inventory must be a list")
        for item in software:
            if not isinstance(item, dict) or not isinstance(item.get("name"), str):
                raise ValueError("invalid OCR provenance software record")
            digest = item.get("sha256")
            if not isinstance(digest, str) or not re.fullmatch(r"[0-9a-fA-F]{64}", digest):
                raise ValueError("OCR provenance software record has no SHA-256")
            name = item["name"]
            components.append({
                "SPDXID": "SPDXRef-OCR-Software-" + _checksum(f"{target}|{name}|{item.get('version')}|{digest}")[:16],
                "name": name, "versionInfo": str(item.get("version", "NOASSERTION")),
                "downloadLocation": "NOASSERTION",
                "licenseConcluded": str(item.get("license", "NOASSERTION")),
                "licenseDeclared": str(item.get("license", "NOASSERTION")),
                "filesAnalyzed": False,
                "checksums": [{"algorithm": "SHA256", "checksumValue": digest.lower()}],
                "comment": "frozen OCR sidecar dependency provenance",
            })
    if len({item["SPDXID"] for item in components}) != len(components):
        raise ValueError("OCR provenance contains duplicate software IDs")
    return components


def _local_runtime_components(manifest: dict[str, Any], target: str) -> list[dict[str, Any]]:
    if manifest.get("schema") != "mpdf-local-runtime-components/1" or manifest.get("target") != target:
        raise ValueError("local runtime manifest target/schema mismatch")
    result = []
    for item in manifest.get("components", []):
        if not re.fullmatch(r"[0-9a-f]{64}", item.get("sha256", "")):
            raise ValueError("local runtime component checksum missing")
        result.append({"SPDXID": "SPDXRef-LocalRuntime-" + _checksum(item["name"])[:16],
                       "name": item["name"], "versionInfo": item["version"],
                       "downloadLocation": item["url"], "licenseDeclared": item["license"],
                       "licenseConcluded": item["license"], "filesAnalyzed": False,
                       "checksums": [{"algorithm": "SHA256", "checksumValue": item["sha256"]}],
                       "comment": item.get("comment", "")})
    if len(result) < 6 or len({item["SPDXID"] for item in result}) != len(result):
        raise ValueError("local runtime component set is incomplete or duplicated")
    return result


def build_sbom(*, project_version: str, target: str, cargo_metadata: dict[str, Any], pnpm_lock: str | None,
               pdfium_manifest: dict[str, Any], node_modules: Path | None = None,
               node_graph: list[dict[str, Any]] | None = None, created: str | None = None,
               ocr_runtime_manifest: dict[str, Any] | Path | None = None,
               local_runtime_manifest: dict[str, Any] | None = None) -> dict[str, Any]:
    node_components, node_relationships = _node_components(pnpm_lock, node_modules, node_graph)
    rust_components = _rust_components(cargo_metadata)
    components = rust_components + node_components
    components.append(_pdfium_component(pdfium_manifest, target))
    local_components = _local_runtime_components(local_runtime_manifest, target) if local_runtime_manifest else []
    components.extend(local_components)
    if ocr_runtime_manifest is not None:
        if isinstance(ocr_runtime_manifest, Path):
            try:
                ocr_runtime_manifest = json.loads(ocr_runtime_manifest.read_text(encoding="utf-8"))
            except (OSError, json.JSONDecodeError) as exc:
                raise ValueError(f"cannot read OCR runtime manifest: {exc}") from exc
        components.extend(_ocr_runtime_components(ocr_runtime_manifest, target))
    components.append(
        {
            "SPDXID": "SPDXRef-MPDF-Processor",
            "name": "Museion PDF",
            "versionInfo": project_version,
            "downloadLocation": "https://github.com/Museion-Project/museion-binarize",
            "licenseConcluded": "MIT OR Apache-2.0",
            "licenseDeclared": "MIT OR Apache-2.0",
            "filesAnalyzed": False,
            "comment": "Project component; OCR runtime is included only when an explicit verified runtime manifest is supplied.",
        }
    )
    components.sort(key=lambda c: (c["name"].lower(), c["versionInfo"], c["SPDXID"]))
    relationships = [
        {"spdxElementId": "SPDXRef-DOCUMENT", "relationshipType": "DESCRIBES", "relatedSpdxElement": "SPDXRef-MPDF-Processor"},
        *_rust_relationships(cargo_metadata), *node_relationships,
    ]
    relationships.extend({"spdxElementId": "SPDXRef-MPDF-Processor", "relationshipType": "DEPENDS_ON", "relatedSpdxElement": item["SPDXID"]} for item in local_components)
    created = created or stable_created()
    if not re.fullmatch(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z", created):
        raise ValueError("created must be an RFC3339 UTC second, e.g. 2026-01-01T00:00:00Z")
    canonical = json.dumps({"created": created, "packages": components, "relationships": relationships}, sort_keys=True, separators=(",", ":"))
    namespace = "https://github.com/Museion-Project/museion-binarize/sbom/" + _checksum(
        f"{project_version}:{target}:{canonical}"
    )[:24]
    document = {
        "spdxVersion": "SPDX-2.3",
        "dataLicense": "CC0-1.0",
        "SPDXID": "SPDXRef-DOCUMENT",
        "name": f"mpdf-{project_version}-{target}",
        "documentNamespace": namespace,
        "creationInfo": {"created": created, "creators": ["Tool: mpdf-sbom-generator"]},
        "packages": components,
        "relationships": relationships,
    }
    validate_sbom(document)
    return document


def validate_sbom(document: dict[str, Any]) -> None:
    """Validate the small SPDX 2.3 contract without third-party packages."""
    if document.get("spdxVersion") != "SPDX-2.3":
        raise ValueError("SBOM must use SPDX-2.3")
    info = document.get("creationInfo")
    created = info.get("created") if isinstance(info, dict) else None
    if not isinstance(created, str) or not re.fullmatch(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z", created):
        raise ValueError("SBOM creationInfo.created must be an RFC3339 UTC second")
    identifiers = [document.get("SPDXID")]
    packages = document.get("packages")
    if not isinstance(packages, list):
        raise ValueError("SBOM packages must be a list")
    identifiers.extend(package.get("SPDXID") if isinstance(package, dict) else None for package in packages)
    if any(not isinstance(identifier, str) for identifier in identifiers) or len(identifiers) != len(set(identifiers)):
        raise ValueError("SBOM SPDXID values must be unique")
    known = set(identifiers)
    for relationship in document.get("relationships", []):
        if relationship.get("spdxElementId") not in known or relationship.get("relatedSpdxElement") not in known:
            raise ValueError("SBOM relationship references an unknown SPDXID")


def stable_created() -> str:
    """Use an explicit reproducible epoch, never wall-clock build time."""
    value = os.environ.get("SOURCE_DATE_EPOCH")
    if value is None:
        try:
            value = subprocess.check_output(["git", "show", "-s", "--format=%ct", "HEAD"], cwd=REPO_ROOT, text=True).strip()
        except (OSError, subprocess.CalledProcessError):
            value = "0"
    return datetime.fromtimestamp(int(value), timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def load_cargo_metadata() -> dict[str, Any]:
    result = subprocess.run(
        ["cargo", "metadata", "--locked", "--format-version", "1"],
        cwd=REPO_ROOT, capture_output=True, text=True, check=True,
    )
    return json.loads(result.stdout)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--created", help="reproducible RFC3339 UTC creation time")
    parser.add_argument("--ocr-runtime-manifest", type=Path,
                        help="verified staged OCR runtime-manifest.json to include")
    parser.add_argument("--local-runtime-manifest", type=Path, help="hash-pinned local bookmark runtime components")
    args = parser.parse_args()
    with (REPO_ROOT / "distribution/pdfium/manifest.toml").open("rb") as f:
        pdfium = tomllib.load(f)
    lock = (REPO_ROOT / "pnpm-lock.yaml").read_text() if (REPO_ROOT / "pnpm-lock.yaml").is_file() else None
    node_graph = load_node_graph(REPO_ROOT / "apps/desktop")
    sbom = build_sbom(project_version=args.version, target=args.target, cargo_metadata=load_cargo_metadata(),
                      pnpm_lock=lock, pdfium_manifest=pdfium,
                      # pnpm's actual package metadata lives in the workspace
                      # store; reachability remains restricted by node_graph.
                      node_modules=REPO_ROOT / "node_modules", node_graph=node_graph,
                      created=args.created, ocr_runtime_manifest=args.ocr_runtime_manifest,
                      local_runtime_manifest=json.loads(args.local_runtime_manifest.read_text()) if args.local_runtime_manifest else None)
    validate_installed_node_graph(node_graph, sbom["packages"], sbom["relationships"])
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(sbom, indent=2, sort_keys=True) + "\n")
    print(f"wrote {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
