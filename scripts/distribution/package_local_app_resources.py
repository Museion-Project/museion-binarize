"""Compose offline local App resources from explicit, hash-bound existing inputs.

The OCR payload stays unchanged. PDFium and declared notices are staged in a new
directory for the App's existing root-resource resolver. No download, execution,
App bundling, signing, installation, or release admission is performed.
"""
import argparse
import json
import shutil
import tempfile
import tomllib
from pathlib import Path

from scripts.ocr.app_mvp_bridge.runtime_contract import verify_inventory
from .package_local_candidate import local_app_overlay, macho_audit, output_guard, sha

TARGET = 'aarch64-apple-darwin'
NOTICES = ('LICENSE-MIT', 'LICENSE-APACHE', 'THIRD_PARTY_LICENSES.md',
           'third_party/pdfium/LICENSE-PDFIUM', 'third_party/pdfium/LICENSE-DISTRIBUTION')


def regular_file(value):
    path = Path(value).absolute()
    if path.is_symlink() or any(p.is_symlink() for p in path.parents):
        raise ValueError('APP_RESOURCE_INPUT_SYMLINK')
    if not path.is_file():
        raise ValueError('APP_RESOURCE_INPUT_REQUIRED')
    return path.resolve()


def app_resources_overlay(repo, local_resources, desktop_resources):
    overlay, inputs = local_app_overlay(repo, local_resources)
    overlay['bundle']['resources'][str(Path(desktop_resources).resolve() / 'pdfium' / '*')] = './'
    return overlay, inputs


def compose(repo, local_resources, local_manifest_sha256, pdfium_library,
            pdfium_manifest, pdfium_manifest_sha256, target, staging, output):
    repo = Path(repo).resolve()
    local_resources = Path(local_resources).absolute()
    if local_resources.is_symlink() or any(p.is_symlink() for p in local_resources.parents):
        raise ValueError('APP_RESOURCE_INPUT_SYMLINK')
    local_resources = local_resources.resolve()
    staging = output_guard(staging, empty=True)
    output = output_guard(output)
    if (staging == output or staging in output.parents or output in staging.parents or
            local_resources == output or local_resources in output.parents or output in local_resources.parents or
            local_resources == staging or local_resources in staging.parents or staging in local_resources.parents):
        raise ValueError('APP_RESOURCE_SCOPE_OVERLAP')
    local_manifest = regular_file(local_resources / 'runtime-manifest.json')
    if sha(local_manifest) != local_manifest_sha256:
        raise ValueError('APP_LOCAL_MANIFEST_HASH')
    inventory = json.loads(local_manifest.read_text())
    local_proof = verify_inventory(local_resources, inventory)
    for name, expected in inventory['code_files'].items():
        if sha(regular_file(repo / name)) != expected:
            raise ValueError('APP_LOCAL_SOURCE_CHANGED')
    pdfium_manifest = regular_file(pdfium_manifest)
    if sha(pdfium_manifest) != pdfium_manifest_sha256:
        raise ValueError('APP_PDFIUM_MANIFEST_HASH')
    manifest = tomllib.loads(pdfium_manifest.read_text())
    if manifest.get('schema') != 'mpdf-pdfium-manifest' or manifest.get('schema_version') != '1.0':
        raise ValueError('APP_PDFIUM_MANIFEST_SCHEMA')
    if target != TARGET:
        raise ValueError('APP_TARGET_UNVERIFIED')
    assets = [a for a in manifest.get('asset', []) if a.get('target_triple') == target]
    if len(assets) != 1:
        raise ValueError('APP_PDFIUM_TARGET_IDENTITY')
    asset = assets[0]
    if (asset.get('library_filename') != 'libpdfium.dylib' or
            asset.get('os') != 'macos' or asset.get('arch') != 'aarch64'):
        raise ValueError('APP_PDFIUM_TARGET_IDENTITY')
    pdfium_library = regular_file(pdfium_library)
    if pdfium_library.name != asset['library_filename'] or sha(pdfium_library) != asset['library_sha256']:
        raise ValueError('APP_PDFIUM_LIBRARY_IDENTITY')
    sources = [(pdfium_library, 'libpdfium.dylib')]
    sources += [(regular_file(repo / name), Path(name).name) for name in NOTICES]
    for path, _ in sources:
        if path == output or output in path.parents or path == staging or staging in path.parents:
            raise ValueError('APP_RESOURCE_SCOPE_OVERLAP')
    source_hashes = {str(path): sha(path) for path, _ in sources}
    source_hashes[str(pdfium_manifest)] = pdfium_manifest_sha256
    overlay, config_inputs = app_resources_overlay(repo, local_resources, output)
    staging.mkdir(parents=True, exist_ok=True)
    work = Path(tempfile.mkdtemp(prefix='local-app-resources-', dir=staging))
    try:
        pdfium_dir = work / 'pdfium'
        pdfium_dir.mkdir()
        for source, name in sources:
            destination = pdfium_dir / name
            shutil.copyfile(source, destination)
            if sha(destination) != source_hashes[str(source)]:
                raise ValueError('APP_RESOURCE_COPY_CHANGED')
        native = macho_audit(pdfium_dir)
        copied = {str(p.relative_to(work)): sha(p) for p in pdfium_dir.iterdir()}
        # Revalidate the external payload and all input files immediately before
        # publication. This is consistency evidence, not atomic file execution.
        if sha(local_manifest) != local_manifest_sha256:
            raise ValueError('APP_LOCAL_MANIFEST_HASH')
        verify_inventory(local_resources, inventory)
        for name, expected in inventory['code_files'].items():
            if sha(regular_file(repo / name)) != expected:
                raise ValueError('APP_LOCAL_SOURCE_CHANGED')
        for name, expected in source_hashes.items():
            if sha(regular_file(name)) != expected:
                raise ValueError('APP_RESOURCE_INPUT_CHANGED')
        current_overlay, current_config_inputs = app_resources_overlay(repo, local_resources, output)
        if current_overlay != overlay or current_config_inputs != config_inputs:
            raise ValueError('APP_CONFIG_CHANGED_DURING_BUILD')
        receipt = dict(schema='local-App-resource-composition/1', state='RESOURCE_RECIPE_CANDIDATE',
                       target=target, local_resources=str(local_resources),
                       local_manifest_sha256=local_manifest_sha256, local_inventory=local_proof,
                       pdfium_manifest_sha256=pdfium_manifest_sha256, pdfium_asset=asset,
                       source_files=source_hashes, copied_files=copied, native=native,
                       config_inputs=config_inputs, composer_sha256=sha(__file__),
                       resource_overlay=overlay, local_payload_modified=False,
                       downloads=0, OCR=0, App_started=False, provider=0, signing_performed=False,
                       actual_App_bundle_verified=False, PDFium_loaded=False,
                       license_obligations_verified=False, declared_notices_copied=True,
                       quality_ready=False, app_admission=False, distribution_ready=False, release_ready=False)
        (work / 'tauri.local-App.overlay.json').write_text(json.dumps(overlay, indent=2) + '\n')
        (work / 'resource-composition.json').write_text(json.dumps(receipt, indent=2) + '\n')
        output.mkdir(parents=True, exist_ok=False)
        for child in work.iterdir():
            shutil.move(str(child), str(output / child.name))
        return dict(state=receipt['state'], output=str(output), copied_files=len(copied),
                    overlay=str(output / 'tauri.local-App.overlay.json'), distribution_ready=False,
                    release_ready=False)
    finally:
        shutil.rmtree(work)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('repo', 'local-resources', 'pdfium-library', 'pdfium-manifest', 'staging', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    for name in ('local-manifest-sha256', 'pdfium-manifest-sha256', 'target'):
        parser.add_argument('--' + name, required=True)
    args = parser.parse_args()
    print(json.dumps(compose(**vars(args))))


if __name__ == '__main__':
    main()
