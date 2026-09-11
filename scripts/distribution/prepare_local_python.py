#!/usr/bin/env python3
"""Fetch the hash-pinned portable Python and install hash-pinned build inputs."""
import argparse, hashlib, json, os, platform, subprocess, tarfile, urllib.request
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--dest',type=Path,required=True);args=parser.parse_args()
    if platform.system()!='Darwin' or platform.machine()!='arm64':raise RuntimeError('macOS arm64 only')
    dest=args.dest.resolve();dest.mkdir(parents=True,exist_ok=False)
    spec=json.loads((ROOT/'distribution/local-runtime/python.json').read_text())
    archive=dest/'python.tar.gz'
    with urllib.request.urlopen(spec['archive_url'],timeout=120) as response,archive.open('wb') as output:
        import shutil
        shutil.copyfileobj(response,output)
    if hashlib.sha256(archive.read_bytes()).hexdigest()!=spec['archive_sha256']:raise ValueError('Python archive checksum mismatch')
    with tarfile.open(archive) as tar:tar.extractall(dest,filter='data')
    python=dest/'python/bin/python3.11';venv=dest/'env'
    subprocess.run([python,'-m','venv',venv],check=True)
    subprocess.run([venv/'bin/python','-m','pip','install','--no-cache-dir','--only-binary=:all:','--require-hashes','-r',ROOT/'distribution/local-runtime/requirements-macos-arm64.lock'],check=True,env=dict(os.environ,PYTHONNOUSERSITE='1'))
    print(venv/'bin/python')
if __name__=='__main__':main()
