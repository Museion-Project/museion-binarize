"""Search real PDF text using canonical Unicode equivalence, preserving diacritics."""
import argparse,json,unicodedata
from pathlib import Path
import fitz

def search(path,query,literal=False):
    needle=query if literal else unicodedata.normalize('NFC',query)
    doc=fitz.open(path)
    return {'query':query,'pdf_query':needle,'normalization':'literal'if literal else 'NFC',
            'hits':[{'page':i+1,'bbox':list(box)}for i,page in enumerate(doc)for box in page.search_for(needle)]}

if __name__=='__main__':
    p=argparse.ArgumentParser(description='Search local PDF; preserves Greek accent/breathing distinctions')
    p.add_argument('pdf',type=Path);p.add_argument('query');p.add_argument('--literal',action='store_true')
    a=p.parse_args();print(json.dumps(search(a.pdf,a.query,a.literal),ensure_ascii=False,indent=2))
