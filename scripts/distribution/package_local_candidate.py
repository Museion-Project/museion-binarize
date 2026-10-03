"""Explicit-input, local-only resource staging. No downloads, implicit relocation, signing,
installation, OCR, or deletion of previous packages. A resource bundle is not an
App acceptance or release claim. New Mach-O closure is inspected on every build.
"""
import argparse
import ast
import hashlib
import json
import os
import shutil
import subprocess
import uuid
from pathlib import Path

from scripts.ocr.app_mvp_bridge.runtime_contract import SOURCE_FILES
COMPONENTS={'python','pymupdf','numpy','opencv-headless','pillow','tesseract','native-closure','apple-helper','noto-sans','tessdata-eng','tessdata-grc','notice'}
MAGICS={b'\xcf\xfa\xed\xfe',b'\xfe\xed\xfa\xcf',b'\xca\xfe\xba\xbe',b'\xbe\xba\xfe\xca',b'\xce\xfa\xed\xfe',b'\xfe\xed\xfa\xce'}
# The approved arm64 local target and current Apple helper require macOS 27.
# This declaration does not certify another OS or a future helper build.
LOCAL_MINIMUM_MACOS='27.0'


def sha(path):
    h=hashlib.sha256()
    with Path(path).open('rb') as f:
        for part in iter(lambda:f.read(1024*1024),b''):h.update(part)
    return h.hexdigest()


def relative(value):
    p=Path(value)
    if p.is_absolute() or '..' in p.parts or str(p)=='.' or str(value)!=p.as_posix():raise ValueError('PACKAGE_PATH_ESCAPE')
    if p.suffix=='.pth' or p.name.startswith(('sitecustomize','usercustomize')):raise ValueError('IMPLICIT_RUNTIME_HOOK')
    return p


def output_guard(path, *, empty=False):
    path=Path(path)
    if path.is_symlink() or any(p.is_symlink() for p in path.parents):raise ValueError('OUTPUT_SYMLINK')
    p=path.resolve()
    if 'evidence' in p.parts or 'research-sessions' in p.parts or 'Applications' in p.parts:raise ValueError('PROTECTED_OUTPUT')
    if p.exists() and (not empty or not p.is_dir() or any(p.iterdir())):raise ValueError('OUTPUT_NOT_NEW_OR_EMPTY')
    return p


def local_app_overlay(repo, output):
    """Remove inherited resource mappings with Tauri's JSON Merge Patch rules.

    A resource object in a config overlay is merged, not replaced. Explicit null
    entries delete default mappings; the final parsed Config contains only the
    declared local payload. These are build inputs, not runtime dependencies.
    """
    repo=Path(repo).resolve();config_root=repo/'apps/desktop/src-tauri'
    inputs=[];inherited=set()
    for name in ('tauri.conf.json5','Tauri.toml','tauri.macos.conf.json5','Tauri.macos.toml'):
        if (config_root/name).exists():raise ValueError('APP_CONFIG_FORMAT_UNSUPPORTED')
    for name in ('tauri.conf.json','tauri.macos.conf.json'):
        path=config_root/name
        if name=='tauri.macos.conf.json' and not path.exists() and not path.is_symlink():continue
        if path.is_symlink() or any(p.is_symlink() for p in path.parents):raise ValueError('APP_CONFIG_SYMLINK')
        if not path.is_file():raise ValueError('APP_BASE_CONFIG_REQUIRED')
        data=json.loads(path.read_text())
        if not isinstance(data,dict) or not isinstance(data.get('bundle',{}),dict):raise ValueError('APP_CONFIG_RESOURCE_SCHEMA')
        resources=data.get('bundle',{}).get('resources')
        if isinstance(resources,dict):
            if any(not isinstance(k,str) or not k or not isinstance(v,str) for k,v in resources.items()):
                raise ValueError('APP_CONFIG_RESOURCE_SCHEMA')
            inherited.update(resources)
        elif resources is not None and (not isinstance(resources,list) or any(not isinstance(v,str) for v in resources)):
            raise ValueError('APP_CONFIG_RESOURCE_SCHEMA')
        inputs.append(dict(path=str(path.relative_to(repo)),sha256=sha(path)))
    mappings={name:None for name in sorted(inherited)}
    mappings[str(Path(output).resolve())]='local-ocr'
    overlay=dict(productName='Museion Local OCR Candidate',identifier='me.mpdf.processor.local-ocr-candidate',
                 bundle=dict(resources=mappings,macOS=dict(minimumSystemVersion=LOCAL_MINIMUM_MACOS)))
    return overlay,inputs


def preflight(repo, freeze, runtime, staging, output):
    repo=Path(repo).resolve();staging=output_guard(staging,empty=True);output=output_guard(output)
    if staging==output or staging in output.parents or output in staging.parents:raise ValueError('OUTPUT_SCOPE_OVERLAP')
    if freeze.get('schema')!='local-code-freeze/1' or runtime.get('schema')!='local-dependency-inputs/1':raise ValueError('INPUT_MANIFEST_SCHEMA')
    files=[];names=set()
    expected=set(SOURCE_FILES)
    if set(freeze['files'])!=expected:raise ValueError('CODE_FREEZE_INCOMPLETE_OR_EXTRA')
    for name,h in freeze['files'].items():
        dest=relative(name);source=(repo/dest).resolve()
        if not source.is_relative_to(repo) or source.is_symlink() or sha(source)!=h:raise ValueError('CODE_SOURCE_HASH_OR_SCOPE')
        files.append(dict(source=str(source),path=str(dest),sha256=h,component='source',provenance='current working tree code freeze'))
        names.add(str(dest))
    for item in runtime['files']:
        dest=relative(item['path']);source=Path(item['source']).resolve(strict=True)
        if item['component'] not in COMPONENTS:raise ValueError('NON_LOCAL_COMPONENT')
        if not source.is_file() or sha(source)!=item['sha256']:raise ValueError('DEPENDENCY_SOURCE_HASH')
        if str(dest) in names:raise ValueError('DUPLICATE_PACKAGE_PATH')
        if dest.parts[0] not in ('python','native','bin','fonts','tessdata','notices','provenance'):raise ValueError('DEPENDENCY_DESTINATION')
        files.append(dict(item,source=str(source)));names.add(str(dest))
    for item in files:
        source=Path(item['source'])
        if source==staging or staging in source.parents or source==output or output in source.parents:raise ValueError('INPUT_OUTPUT_OVERLAP')
    for item in runtime.get('remove_rpaths',[]):
        name=str(relative(item['path']))
        match=next((f for f in files if f['path']==name and f['component']!='source'),None)
        if not match or item['source_sha256']!=match['sha256']:raise ValueError('RELOCATION_SOURCE_IDENTITY')
        if not item['rpaths'] or any(not p.startswith('/') for p in item['rpaths']):raise ValueError('RELOCATION_SCOPE')
    tree=ast.parse((repo/'scripts/ocr/mvp/core.py').read_text())
    version=next(ast.literal_eval(n.value) for n in tree.body if isinstance(n,ast.Assign)
                 and any(isinstance(t,ast.Name) and t.id=='CONFIG_VERSION' for t in n.targets))
    if freeze.get('config_version')!=version:raise ValueError('PACKAGE_CONFIG_VERSION')
    tree=ast.parse((repo/'scripts/ocr/mvp/store.py').read_text())
    exporter=next(ast.literal_eval(n.value) for n in tree.body if isinstance(n,ast.Assign)
                  and any(isinstance(t,ast.Name) and t.id=='EXPORTER_VERSION' for t in n.targets))
    if freeze.get('exporter_version')!=exporter:raise ValueError('PACKAGE_EXPORTER_VERSION')
    contract=runtime['runtime']
    for key in ('python','apple_helper','tesseract','font'):
        if str(relative(contract[key])) not in names:raise ValueError('RUNTIME_CONTRACT_MISSING')
    for language in ('eng','grc'):
        if str(relative(contract['tessdata'])/(language+'.traineddata')) not in names:raise ValueError('TESSDATA_CONTRACT_MISSING')
    # Raw source and declared binary provenance are not equivalent.
    helper=runtime.get('helper_build')
    if not helper or helper.get('schema')!='apple-helper-build/1':raise ValueError('HELPER_BUILD_PROVENANCE_MISSING')
    hfile=next(f for f in files if f['path']==contract['apple_helper'])
    if helper['binary_sha256']!=hfile['sha256'] or not helper.get('compiler_version') or not helper.get('sdk'):
        raise ValueError('HELPER_BUILD_IDENTITY')
    if sha(helper['source_path'])!=helper['source_sha256'] or not helper.get('command'):
        raise ValueError('HELPER_SOURCE_IDENTITY')
    return files,version,staging,output


def macho_audit(root):
    records=[]
    for path in sorted(root.rglob('*')):
        if not path.is_file():continue
        with path.open('rb') as f:magic=f.read(4)
        if magic not in MAGICS:continue
        def run(args):return subprocess.check_output(args,text=True)
        deps=[s.strip().split(' (')[0] for s in run(['/usr/bin/otool','-L',str(path)]).splitlines()[1:]]
        own=run(['/usr/bin/otool','-D',str(path)]).splitlines()[1:]
        load=run(['/usr/bin/otool','-l',str(path)]).splitlines();rpaths=[]
        for i,line in enumerate(load):
            if line.strip()=='cmd LC_RPATH':
                rpaths.extend(s.strip().split('path ',1)[1].split(' (offset',1)[0] for s in load[i+1:i+5] if s.strip().startswith('path '))
        def resolve(value):
            if value=='@loader_path':return path.parent.resolve()
            if value.startswith('@loader_path/'):return (path.parent/value.split('/',1)[1]).resolve()
            # Executable-relative closure must be unambiguous; don't guess which
            # executable will load a library. Such inputs need explicit relocation.
            raise ValueError('UNSUPPORTED_OR_EXTERNAL_MACHO_PATH: '+value)
        local_rpaths=[resolve(r) for r in rpaths]
        if any(not r.is_relative_to(root) for r in local_rpaths):raise ValueError('MACHO_RPATH_ESCAPE')
        for dep in deps:
            if dep in own or dep.startswith(('/usr/lib/','/System/Library/')):continue
            if dep.startswith('@rpath/'):
                candidates=[r/dep.split('/',1)[1] for r in local_rpaths if (r/dep.split('/',1)[1]).is_file()]
                if len(candidates)!=1:raise ValueError('MACHO_RPATH_AMBIGUOUS')
                target=candidates[0].resolve()
            else:target=resolve(dep)
            if not target.is_relative_to(root) or not target.is_file():raise ValueError('MACHO_DEPENDENCY_ESCAPE_OR_MISSING')
        records.append(dict(path=str(path.relative_to(root)),sha256=sha(path),dependencies=deps,rpaths=rpaths))
    return records


def package(repo, freeze_path, runtime_path, staging, output, *, check_only=False):
    freeze=json.loads(Path(freeze_path).read_text());runtime=json.loads(Path(runtime_path).read_text())
    files,version,staging,output=preflight(repo,freeze,runtime,staging,output)
    overlay,app_config_inputs=local_app_overlay(repo,output)
    if check_only:return dict(state='PREFLIGHT_PASS',files=len(files),config_version=version,
                              app_config_inputs=app_config_inputs,distribution_ready=False)
    staging.mkdir(parents=True,exist_ok=True)
    work=staging/('prepare-'+uuid.uuid4().hex);work.mkdir(mode=0o700)
    try:
        for item in files:
            dest=work/item['path'];dest.parent.mkdir(parents=True,exist_ok=True)
            shutil.copy2(item['source'],dest)
            if sha(dest)!=item['sha256'] or sha(item['source'])!=item['sha256']:raise ValueError('COPY_OR_SOURCE_CHANGED')
        for name in ('scripts/__init__.py','scripts/ocr/__init__.py'):
            (work/name).touch(exist_ok=False)
        mutations=[]
        for item in runtime.get('remove_rpaths',[]):
            dest=work/item['path'];before=sha(dest)
            if before!=item['source_sha256']:raise ValueError('RELOCATION_COPY_CHANGED')
            args=['/usr/bin/install_name_tool']
            for rpath in item['rpaths']:args.extend(['-delete_rpath',rpath])
            args.append(str(dest))
            proc=subprocess.run(args,capture_output=True,text=True,check=True)
            # This is a bounded mutation of this invocation's new copy, not
            # developer/release signing. Signatures need their own acceptance.
            verify=subprocess.run(['/usr/bin/codesign','--verify',str(dest)],capture_output=True,text=True)
            mutations.append(dict(path=item['path'],source_sha256=before,output_sha256=sha(dest),
                                  command=args,stdout=proc.stdout,stderr=proc.stderr,
                                  signature_verify_returncode=verify.returncode,
                                  signature_verify_stderr=verify.stderr,signing_performed=False))
        closure=macho_audit(work)
        manifest=dict(runtime['runtime'],schema='museion-local-runtime/1',config_version=version,
                      exporter_version=freeze['exporter_version'],
                      code_files=freeze['files'],
                      runtime_files={str(p.relative_to(work)):sha(p) for p in work.rglob('*') if p.is_file()},
                      code_freeze_sha256=sha(freeze_path),dependency_manifest_sha256=sha(runtime_path),
                      modes=['local'],distribution_ready=False,release_ready=False)
        (work/'runtime-manifest.json').write_text(json.dumps(manifest,indent=2))
        from scripts.ocr.app_mvp_bridge.runtime_contract import verify_inventory
        verify_inventory(work,manifest)
        inventory=[dict(path=str(p.relative_to(work)),sha256=sha(p),bytes=p.stat().st_size) for p in sorted(work.rglob('*')) if p.is_file()]
        report=dict(schema='local-resource-build/1',state='RESOURCE_CANDIDATE',files=inventory,inputs=files,
                    helper_build=runtime['helper_build'],components=runtime.get('components',[]),macho=closure,
                    native_mutations=mutations,signing_performed=False,packager_sha256=sha(__file__),
                    app_config_inputs=app_config_inputs,
                    app_resource_policy='JSON Merge Patch deletes inherited mappings; explicit local payload only',
                    signatures_unverified=True,
                    source_freeze=freeze,distribution_ready=False,release_ready=False,
                    unresolved=runtime.get('unresolved',[])+['actual App/runtime identity and independent Mac acceptance pending'])
        # Directory creation is exclusive; never replace an existing output even
        # if a competing process creates it between preflight and publication.
        current_overlay,current_inputs=local_app_overlay(repo,output)
        if current_overlay!=overlay or current_inputs!=app_config_inputs:raise ValueError('APP_CONFIG_CHANGED_DURING_BUILD')
        output.mkdir(parents=True,exist_ok=False)
        for child in work.iterdir():shutil.move(str(child),str(output/child.name))
        with (output/'resource-build.json').open('x') as f:json.dump(report,f,indent=2)
        with (output/'tauri.generated.overlay.json').open('x') as f:json.dump(overlay,f,indent=2)
        return dict(state=report['state'],output=str(output),files=len(inventory),distribution_ready=False)
    finally:
        # This invocation's private work directory only; no old package cleanup.
        shutil.rmtree(work)


def main():
    p=argparse.ArgumentParser(description=__doc__)
    for name in ('repo','code-freeze','runtime-manifest','staging','output'):p.add_argument('--'+name,type=Path,required=True)
    p.add_argument('--check-only',action='store_true');a=p.parse_args()
    print(json.dumps(package(a.repo,a.code_freeze,a.runtime_manifest,a.staging,a.output,check_only=a.check_only)))


if __name__=='__main__':main()
