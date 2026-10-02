"""Reuse the existing strict bookmark writer with append-only serialization.
MuPDF's normal save discards original ObjStm containers even with garbage=0.
The adapter transfers only newly allocated outline objects and Catalog/Outlines
onto a byte-identical temporary source copy, then performs an incremental save.
All existing writer source-object/stream/pixel/destination checks remain active.
"""
import shutil
import re
import fitz


class Document:
    def __init__(self, inner):
        self.inner=inner
        self.original_count=inner.xref_length()

    def __getattr__(self,name):return getattr(self.inner,name)
    def __len__(self):return len(self.inner)
    def __iter__(self):return iter(self.inner)
    def __getitem__(self,index):return self.inner[index]

    def save(self,output,**options):
        source=self.inner.name
        shutil.copyfile(source,output)
        # MuPDF incremental serialization can inject Catalog/PageMode for scans
        # with embedded files. Append the explicitly changed dictionary and new
        # outline objects ourselves; the existing strict writer reopens and
        # validates every original object/stream/page and destination below.
        with fitz.open(source) as original:
            if original.is_encrypted or not original.can_save_incrementally():raise ValueError('source does not support unencrypted append-only save')
            catalog=self.inner.pdf_catalog()
            if self.inner.xref_get_key(catalog,'Outlines')[0]!='xref':raise ValueError('missing new outlines reference')
            trailer=original.pdf_trailer()
        with open(source,'rb') as handle:source_bytes=handle.read()
        markers=list(re.finditer(rb'startxref\s+(\d+)\s+%%EOF',source_bytes))
        if not markers:raise ValueError('original startxref missing')
        previous=int(markers[-1][1])
        trailer=re.sub(r'/(?:Size|Prev|XRefStm)\s+\d+','',trailer)
        trailer=trailer.rstrip()[:-2]+f' /Size {self.inner.xref_length()} /Prev {previous} >>'
        refs=[catalog]+list(range(self.original_count,self.inner.xref_length()))
        with open(output,'ab') as handle:
            handle.write(b'\n');offsets={}
            for xref in refs:
                offsets[xref]=handle.tell()
                handle.write(f'{xref} 0 obj\n{self.inner.xref_object(xref)}\nendobj\n'.encode('ascii'))
            offset=handle.tell();handle.write(b'xref\n')
            for xref in sorted(offsets):handle.write(f'{xref} 1\n{offsets[xref]:010d} 00000 n \n'.encode('ascii'))
            handle.write(f'trailer\n{trailer}\nstartxref\n{offset}\n%%EOF\n'.encode('ascii'))
        with open(source,'rb') as before,open(output,'rb') as after:
            while True:
                block=before.read(1024*1024)
                if not block:break
                if after.read(len(block))!=block:raise ValueError('original PDF byte prefix changed')


class Module:
    def __getattr__(self,name):return getattr(fitz,name)
    def open(self,*args,**kwargs):return Document(fitz.open(*args,**kwargs))


def export(writer,table,output,cancelled=None):
    original=writer.fitz
    writer.fitz=Module()
    try:
        result=writer.export(table,output,cancelled=cancelled)
        result['serialization']='append-only; original complete PDF bytes preserved as prefix'
        return result
    finally:
        writer.fitz=original
