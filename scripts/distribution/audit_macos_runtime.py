"""Audit every Mach-O in a frozen runtime before declaring its OS support."""
import hashlib, json, subprocess
from pathlib import Path

MAGIC={b'\xcf\xfa\xed\xfe',b'\xfe\xed\xfa\xcf',b'\xca\xfe\xba\xbe',b'\xbe\xba\xfe\xca',b'\xca\xfe\xba\xbf'}

def audit(root):
    root=Path(root);records=[]
    for path in sorted(root.rglob('*')):
        if path.is_symlink() or not path.is_file():continue
        with path.open('rb') as f:head=f.read(4)
        if head not in MAGIC:continue
        relative=str(path.relative_to(root))
        arch=subprocess.check_output(['lipo','-archs',str(path)],text=True).strip().split()
        if arch!=['arm64']:raise ValueError(f'Unexpected architectures: {relative}: {arch}')
        lines=subprocess.check_output(['otool','-l',str(path)],text=True).splitlines();minimum=None
        for i,line in enumerate(lines):
            if line.strip()=='cmd LC_BUILD_VERSION':
                minimum=next((x.strip().split()[-1] for x in lines[i:i+8] if x.strip().startswith('minos ')),None)
            elif line.strip()=='cmd LC_VERSION_MIN_MACOSX':
                minimum=next((x.strip().split()[-1] for x in lines[i:i+6] if x.strip().startswith('version ')),None)
        optional=relative in {'_internal/hierarchy_models/apple/check-model','_internal/hierarchy_models/apple/hierarchy-worker'}
        if minimum is None or tuple(map(int,minimum.split('.')[:2]))>((27,0) if optional else (13,0)):
            raise ValueError(f'Unsupported minimum OS for {relative}: {minimum}')
        dependencies=[]
        for line in subprocess.check_output(['otool','-L',str(path)],text=True).splitlines()[1:]:
            dep=line.strip().split(' (',1)[0];dependencies.append(dep)
            if dep.startswith('/') and not dep.startswith(('/usr/lib/','/System/Library/')):
                raise ValueError(f'External native dependency: {relative}: {dep}')
        records.append(dict(path=relative,sha256=hashlib.sha256(path.read_bytes()).hexdigest(),architectures=arch,minimum_macos=minimum,optional_apple_hierarchy=optional,dependencies=dependencies))
    if not records:raise ValueError('No native runtime files found')
    return dict(schema='mpdf-macos-runtime-closure/1',minimum_macos='13.0',architecture='arm64',files=records)

if __name__=='__main__':
    import argparse
    p=argparse.ArgumentParser();p.add_argument('root',type=Path);p.add_argument('--output',type=Path);a=p.parse_args();value=audit(a.root)
    if a.output:a.output.write_text(json.dumps(value,indent=2)+'\n')
    else:print(json.dumps(value,indent=2))
