"""Prepare local candidate resources from approved existing inputs; no install/download."""
import argparse,hashlib,json,shutil
from pathlib import Path

def main():
 p=argparse.ArgumentParser();p.add_argument('--resource-root',required=True);p.add_argument('--external-config',required=True);p.add_argument('--repo',required=True);a=p.parse_args()
 repo=Path(a.repo).resolve();root=Path(a.resource_root).resolve();external=json.loads(Path(a.external_config).read_text());root.mkdir(parents=True,exist_ok=True)
 groups={
  'local':['scripts/ocr/app_mvp_bridge','scripts/ocr/mvp','scripts/ocr/free_local'],
  'critical-edition':['scripts/ocr/edition_mvp','scripts/ocr/greek_router'],
  'paid':['scripts/ocr/paid_mvp'],
  'paid-contents':['scripts/bookmarks/paid_mvp','scripts/bookmarks/local'],
 }
 for paths in groups.values():
  for path in paths:shutil.copytree(repo/path,root/path,dirs_exist_ok=True,ignore=shutil.ignore_patterns('__pycache__','test_*.py','tests','*.pyc'))
 assets={'bin/apple-accurate':external['local']['apple_helper'],'fonts/NotoSans-Regular.ttf':external['local']['font'],'fonts/OFL.txt':str(repo/'crates/mpdf-core/assets/fonts/OFL.txt')}
 for name,src in assets.items():
  target=root/name;target.parent.mkdir(parents=True,exist_ok=True)
  if Path(src).is_file():shutil.copy2(src,target)
 tess=Path(external['tessdata'])
 for name in ['grc.traineddata','eng.traineddata','LICENSE']:
  if (tess/name).exists():(root/'tessdata').mkdir(exist_ok=True);shutil.copy2(tess/name,root/'tessdata'/name)
 for name in ['configs','tessconfigs']:
  if (tess/name).is_dir():shutil.copytree(tess/name,root/'tessdata'/name,dirs_exist_ok=True)
 # Preserve module package shape; namespace imports work without modifying branch files.
 for path in ['scripts','scripts/ocr','scripts/bookmarks']:(root/path/'__init__.py').touch()
 manifest=dict(schema_version=1,distribution_ready=False,external_development_runtime=True,modes=groups,files=[],unclosed=['Python + native/standard-library closure','External Tesseract + 15 native libraries','CLLG cached venv/model + declared dependency conflicts','PyMuPDF redistribution license review','helper/model/tessdata complete provenance/notice','independent-machine run'],local_loads_cllg=False)
 for f in root.rglob('*'):
  if f.is_file():manifest['files'].append(dict(path=str(f.relative_to(root)),sha256=hashlib.sha256(f.read_bytes()).hexdigest(),bytes=f.stat().st_size))
 (root/'resource-manifest.json').write_text(json.dumps(manifest,indent=2))
 config=dict(external);config['package_root']=str(root);config['tessdata']=str(root/'tessdata');config['external_development_runtime']=True
 for mode in ['local','critical-edition']:
  config[mode]=dict(config[mode],apple_helper=str(root/'bin/apple-accurate'),font=str(root/'fonts/NotoSans-Regular.ttf'),fallback_font_paths=['/System/Library/Fonts/Times.ttc'])
 (root/'development-runtime.json').write_text(json.dumps(config,indent=2))
 print(json.dumps(dict(resource_root=str(root),manifest=str(root/'resource-manifest.json'),runtime_config=str(root/'development-runtime.json'),distribution_ready=False)))
if __name__=='__main__':main()
