#!/usr/bin/env python3
"""User entrypoint: frozen Surya -> broker -> the existing Rust compositor."""
import argparse,os,subprocess,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
def main():
    p=argparse.ArgumentParser();p.add_argument('image',type=Path);p.add_argument('output',type=Path);p.add_argument('--page-index',type=int,default=0);p.add_argument('--broker',default='http://127.0.0.1:8766');p.add_argument('--config-dir',type=Path,default=Path.home()/'.config/museion-binarize/broker');p.add_argument('--model-dir',type=Path,default=Path.home()/'Library/Caches/datalab/models/text_detection/2025_05_07');a=p.parse_args()
    overlay=ROOT/'.runtime/surya-compat'
    if not overlay.is_dir():p.error('Run scripts/ocr/broker/setup_runtime.sh first')
    env={**os.environ,'PYTHONPATH':str(overlay)}
    output=a.output.resolve();geometry=output.with_suffix('.geometry.json');image=a.image.resolve()
    if output.exists() or geometry.exists():p.error('Output or geometry file exists; choose a new output path')
    output.parent.mkdir(parents=True,exist_ok=True)
    # Caller selects the frozen Python environment. This process never reads the Gemini key.
    subprocess.run([sys.executable,str(Path(__file__).with_name('surya_geometry.py')),str(image),str(geometry),'--page-index',str(a.page_index),'--model-dir',str(a.model_dir)],check=True,env=env)
    subprocess.run(['cargo','run','--quiet','-p','mpdf-api-client','--example','surya_broker_page','--',str(image),str(geometry),str(output),a.broker,str(a.config_dir/'client.token')],cwd=ROOT,check=True)
if __name__=='__main__':main()
