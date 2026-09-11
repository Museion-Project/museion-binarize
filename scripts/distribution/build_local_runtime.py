#!/usr/bin/env python3
"""Build the macOS arm64 local runtime from a pinned, isolated Python environment."""
import hashlib, importlib.metadata, json, os, platform, shutil, subprocess, sys
from pathlib import Path

ROOT=Path(__file__).resolve().parents[2]
PINNED={'pypdf':'6.18.1','pypdfium2':'5.13.0','Pillow':'11.3.0','pyinstaller':'6.16.0','altgraph':'0.17.5','macholib':'1.16.4','packaging':'26.3','pyinstaller-hooks-contrib':'2026.7','setuptools':'84.0.0'}

def main():
    if sys.platform!='darwin' or platform.machine()!='arm64':raise RuntimeError('macOS arm64 build only')
    if platform.python_version()!='3.11.16':raise RuntimeError('Use the pinned portable Python 3.11.16 build')
    for name,version in PINNED.items():
        if importlib.metadata.version(name)!=version:raise RuntimeError(f'{name} must be {version}')
    sdk=subprocess.check_output(['xcrun','--sdk','macosx','--show-sdk-version'],text=True).strip()
    if int(sdk.split('.')[0])<27:raise RuntimeError('Building Apple image hierarchy requires macOS SDK 27 or later; select the matching DEVELOPER_DIR')
    work=ROOT/'.release-runtime-build';out=ROOT/'.release-runtime'
    if out.exists():raise FileExistsError('Use a fresh runtime output directory')
    assets=work/'assets';assets.mkdir(parents=True,exist_ok=True)
    local=assets/'bookmarks';local.mkdir(exist_ok=True)
    for name in ['desktop_bridge','bookmarks','pdf_backend','outline_writer','pagination','numeric_lane']:
        shutil.copy2(ROOT/f'scripts/bookmarks/local/{name}.py',local/f'{name}.py')
    for folder,names in [('hierarchy_models',['desktop_bridge','manager']),('hierarchy_foundation_models',['hierarchy'])]:
        target=assets/folder;target.mkdir(exist_ok=True)
        for name in names:shutil.copy2(ROOT/f'scripts/bookmarks/{folder}/{name}.py',target/f'{name}.py')
    subprocess.run(['xcrun','swiftc','-O','-target','arm64-apple-macos13.0','-module-cache-path',str(work/'swift-modules'),str(ROOT/'scripts/bookmarks/local/vision_fast.swift'),'-o',str(local/'vision-fast')],check=True)
    apple=assets/'hierarchy_models/apple';apple.mkdir(exist_ok=True)
    subprocess.run(['xcrun','swiftc','-parse-as-library','-target','arm64-apple-macos26.0','-module-cache-path',str(work/'swift-modules'),str(ROOT/'scripts/bookmarks/hierarchy_models/check_model.swift'),'-o',str(apple/'check-model')],check=True)
    subprocess.run([sys.executable,str(ROOT/'scripts/bookmarks/hierarchy_foundation_models/build_worker.py'),str(apple/'hierarchy-worker')],check=True)
    shutil.rmtree(apple/'module-cache')
    shutil.move(str(apple/'hierarchy-worker.build.json'),str(work/'hierarchy-worker.build.json'))
    notices=assets/'licenses';notices.mkdir(exist_ok=True)
    for name in ['LICENSE-MIT','LICENSE-APACHE','THIRD_PARTY_LICENSES.md']:shutil.copy2(ROOT/name,notices/name)
    shutil.copytree(ROOT/'distribution/local-runtime/python-licenses',notices/'python-native-dependencies')
    for name in ['pypdf','pypdfium2','Pillow','pyinstaller']:
        dist=importlib.metadata.distribution(name)
        for file in dist.files or []:
            if any(term in str(file).lower() for term in ('license','copying','notice')):
                src=Path(dist.locate_file(file))
                if src.is_file():
                    dest=notices/name/str(file);dest.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(src,dest)
    import sysconfig
    python_license=Path(sys.base_prefix)/'lib/python3.11/LICENSE.txt'
    if not python_license.is_file():raise FileNotFoundError('Python runtime license missing')
    shutil.copy2(python_license,notices/'LICENSE-PYTHON.txt')
    command=[sys.executable,'-m','PyInstaller','--noconfirm','--clean','--onedir','--name','museion-local','--distpath',str(out),'--workpath',str(work/'pyinstaller'),'--specpath',str(work),'--collect-all','pypdfium2','--collect-all','pypdfium2_raw','--hidden-import','pypdf','--hidden-import','PIL.Image','--hidden-import','PIL.PngImagePlugin']
    for folder in ['bookmarks','hierarchy_models','hierarchy_foundation_models']:
        command+=['--paths',str(assets/folder)]
    for module in ['bookmarks','pdf_backend','outline_writer','numeric_lane','pagination','manager','hierarchy']:command+=['--hidden-import',module]
    for folder in assets.iterdir():command+=['--add-data',f'{folder}:{folder.name}']
    command+=[str(ROOT/'scripts/distribution/local_runtime_entry.py')]
    env=dict(os.environ,PYINSTALLER_CONFIG_DIR=str(work/'pyinstaller-cache'))
    subprocess.run(command,check=True,env=env)
    from audit_macos_runtime import audit
    closure=audit(out/'museion-local')
    (out/'native-closure.json').write_text(json.dumps(closure,indent=2)+'\n')
    binary=out/'museion-local/museion-local'
    result=json.loads(subprocess.check_output([binary,'--self-check'],text=True))
    receipt=dict(schema='mpdf-local-runtime-build/1',python_provenance=json.loads((ROOT/'distribution/local-runtime/python.json').read_text()),dependencies=PINNED,self_check=result,files={str(p.relative_to(out)):hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(out.rglob('*')) if p.is_file()})
    (out/'build-receipt.json').write_text(json.dumps(receipt,indent=2)+'\n')
    print(json.dumps(result))
if __name__=='__main__':main()
