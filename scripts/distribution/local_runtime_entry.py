"""Frozen local PDF adapter: a bounded dispatcher, never arbitrary script execution."""
import json
from pathlib import Path
import runpy
import sys


def main():
    root=Path(getattr(sys,'_MEIPASS',Path(__file__).resolve().parent))
    if sys.argv[1:]==['--self-check']:
        import pypdf, pypdfium2, PIL
        print(json.dumps(dict(python=sys.version.split()[0],pypdf=pypdf.__version__,pdfium=str(pypdfium2.PDFIUM_INFO),pillow=PIL.__version__,frozen=bool(getattr(sys,'frozen',False)))))
        return
    if len(sys.argv)!=4:raise ValueError('Expected adapter, operation and request path')
    adapter,verb,request=sys.argv[1:]
    allowed={'bookmarks/desktop_bridge.py':{'prepare','analyze','finish','save'},'hierarchy_models/desktop_bridge.py':{'models_status','hierarchy'}}
    if adapter not in allowed or verb not in allowed[adapter]:raise ValueError('Unsupported local operation')
    for folder in ('bookmarks','hierarchy_models','hierarchy_foundation_models'):sys.path.insert(0,str(root/folder))
    sys.argv=[str(root/adapter),verb,request]
    runpy.run_path(str(root/adapter),run_name='__main__')

if __name__=='__main__':
    try:main()
    except Exception as error:
        print(str(error),file=sys.stderr)
        raise SystemExit(1)
