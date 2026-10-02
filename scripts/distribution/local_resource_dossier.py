"""Inspect an existing local resource candidate without executing its binaries.

Inventory hashes, embedded license text and installation paths are different
evidence. This dossier makes unresolved provenance explicit; it grants no legal,
quality, App or release admission and never changes the inspected package.
"""
import argparse
from collections import Counter
from email.parser import Parser
import hashlib
import json
from pathlib import Path
import struct

from scripts.ocr.app_mvp_bridge.runtime_contract import verify_inventory

MAGICS={b'\xcf\xfa\xed\xfe',b'\xfe\xed\xfa\xcf',b'\xce\xfa\xed\xfe',b'\xfe\xed\xfa\xce',b'\xca\xfe\xba\xbe',b'\xbe\xba\xfe\xca'}


def sha(path):return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def font_names(path):
    """Read literal SFNT name records; these do not identify an upstream blob."""
    data=Path(path).read_bytes()
    if len(data)<12 or data[:4] not in (b'\x00\x01\x00\x00',b'OTTO'):
        return dict(state='UNKNOWN',reason='unsupported font container')
    count=struct.unpack_from('>H',data,4)[0];name=None
    if 12+count*16>len(data):raise ValueError('FONT_TABLE_DIRECTORY_TRUNCATED')
    for i in range(count):
        tag,_,offset,length=struct.unpack_from('>4sIII',data,12+i*16)
        if offset+length>len(data):raise ValueError('FONT_TABLE_TRUNCATED')
        if tag==b'name':name=data[offset:offset+length]
    if name is None:return dict(state='UNKNOWN',reason='font name table absent')
    if len(name)<6:raise ValueError('FONT_NAME_TABLE_TRUNCATED')
    _,count,storage=struct.unpack_from('>HHH',name,0)
    if 6+count*12>len(name):raise ValueError('FONT_NAME_RECORDS_TRUNCATED')
    fields={0:'copyright',1:'family',2:'style',3:'unique_id',4:'full_name',5:'version',
            6:'postscript_name',8:'manufacturer',11:'vendor_url',13:'license_literal',14:'license_url'}
    records=[]
    for i in range(count):
        platform,encoding,language,key,length,offset=struct.unpack_from('>6H',name,6+i*12)
        if storage+offset+length>len(name):raise ValueError('FONT_NAME_STRING_TRUNCATED')
        if key not in fields:continue
        raw=name[storage+offset:storage+offset+length]
        try:text=raw.decode('utf-16-be' if platform in (0,3) else 'mac_roman')
        except UnicodeDecodeError:continue
        records.append(dict(field=fields[key],platform=platform,encoding=encoding,language=language,literal=text))
    return dict(state='EMBEDDED_METADATA_OBSERVED',records=records,upstream_blob_identity='UNKNOWN')


def inspect(resource_root,content_provenance=()):
    root=Path(resource_root).resolve();manifest_path=root/'runtime-manifest.json';manifest_sha256=sha(manifest_path)
    manifest=json.loads(manifest_path.read_text());inventory=verify_inventory(root,manifest)
    metadata=[];licenses=[];fonts=[];native=[];references={}
    for file in content_provenance:
        record=json.loads(Path(file).read_text());name=record.get('component_path')
        if name in references:raise ValueError('DUPLICATE_CONTENT_PROVENANCE')
        if (record.get('schema')!='font-upstream-content-provenance/1' or record.get('state')!='CONTENT_MATCH'
            or name not in manifest['runtime_files'] or not name.startswith('fonts/')
            or record.get('packaged_sha256')!=manifest['runtime_files'][name]
            or record.get('upstream_sha256')!=record['packaged_sha256']
            or sha(record['upstream_file'])!=record['upstream_sha256']
            or not record.get('commit') or not record.get('url')):
            raise ValueError('CONTENT_PROVENANCE_IDENTITY')
        references[name]=dict(record=record,record_path=str(Path(file).resolve()),record_sha256=sha(file),
                              evidence_kind='fixed official reference bytes; original installation event unverified')
    for name,expected in sorted(manifest['runtime_files'].items()):
        path=root/name
        if sha(path)!=expected:raise ValueError('RESOURCE_CHANGED_DURING_DOSSIER: '+name)
        if name.endswith('.dist-info/METADATA'):
            message=Parser().parsestr(path.read_text())
            folder=path.parent
            notices=[dict(path=str(p.relative_to(root)),sha256=sha(p)) for p in folder.rglob('*')
                     if p.is_file() and any(k in p.name.casefold() for k in ('license','copying','copyright','notice'))]
            metadata.append(dict(name=message.get('Name'),version=message.get('Version'),
                                 license_expression=message.get('License-Expression'),license_literal=message.get('License'),
                                 project_urls=message.get_all('Project-URL',[]),metadata_path=name,
                                 metadata_sha256=expected,license_files=notices,original_installation_provenance='UNKNOWN'))
        if name.startswith('notices/'):
            licenses.append(dict(path=name,sha256=expected,bytes=path.stat().st_size,
                                 evidence_kind='bundled literal notice; component obligations not automatically reconciled'))
        if name.startswith('fonts/'):
            fonts.append(dict(path=name,sha256=expected,metadata=font_names(path),
                              upstream_content=references.get(name,dict(state='UNKNOWN')),
                              original_installation_provenance='UNKNOWN'))
        with path.open('rb') as file:magic=file.read(4)
        if magic in MAGICS:native.append(dict(path=name,sha256=expected,original_build_provenance='UNKNOWN'))
    # Inventory validation is repeated after inspection; no loaded module or
    # signature claim follows from this static read.
    after=verify_inventory(root,manifest)
    if sha(manifest_path)!=manifest_sha256:raise ValueError('RESOURCE_MANIFEST_CHANGED_DURING_DOSSIER')
    unresolved=[
        'PyMuPDF/MuPDF distribution license decision and exact combined-work/source obligations',
        'original native build/install provenance; embedded metadata is not upstream source proof',
        'all dependency notice/obligation correspondence',
        'final quality/exporter/App candidate; final signed package and independent Mac Release Gate']
    if any(font['path'] not in references for font in fonts):unresolved.append('font upstream blob identity')
    return dict(schema='local-resource-dossier/1',state='REVIEW_DRAFT',resource_root=str(root),
                manifest_sha256=sha(manifest_path),inventory_before=inventory,inventory_after=after,
                code_files=manifest['code_files'],runtime_files=len(manifest['runtime_files']),
                metadata=metadata,notices=licenses,fonts=fonts,native_objects=native,
                grouped_native_paths=dict(Counter(str(Path(n['path']).parent) for n in native)),
                unresolved=unresolved,
                executed_binaries=0,new_ocr_calls=0,new_reader_calls=0,provider_calls=0,
                signing_performed=False,legal_admission=False,quality_ready=False,
                app_admission=False,distribution_ready=False,release_ready=False)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--resource-root',type=Path,required=True);parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--content-provenance',type=Path,action='append',default=[])
    args=parser.parse_args()
    if args.output.resolve().is_relative_to(args.resource_root.resolve()):raise ValueError('DOSSIER_OUTPUT_INSIDE_RESOURCE')
    result=inspect(args.resource_root,args.content_provenance)
    with args.output.open('x') as output:json.dump(result,output,ensure_ascii=False,indent=2)
    print(json.dumps(dict(state=result['state'],runtime_files=result['runtime_files'],
                          metadata=len(result['metadata']),native_objects=len(result['native_objects']),
                          notices=len(result['notices']),distribution_ready=False)))


if __name__=='__main__':main()
