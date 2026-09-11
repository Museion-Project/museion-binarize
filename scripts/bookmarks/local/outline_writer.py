"""pypdf outline serializer with a classic incremental cross-reference table.

pypdf 6.18.1's incremental xref stream omits its own cross-reference entry
and can be emitted into an unchanged PDF 1.3 header. A classic table avoids
both problems and retains the original PDF version and byte prefix. This
adapter is limited to the existing unencrypted bookmark-only operation.
"""
from pypdf import PdfWriter
from pypdf.generic import DictionaryObject, IndirectObject, NameObject, NumberObject


class OutlineWriter(PdfWriter):
    def _write_increment(self, stream):
        if self._reader.is_encrypted:
            raise ValueError('目录处理暂不支持加密 PDF，请先保存不加密副本。')
        root=self._reader.trailer.raw_get('/Root')
        positions={}
        for ref in self.list_objects_in_increment():
            if ref.idnum<=len(self._original_hash) and ref.idnum!=root.idnum:
                raise ValueError(f'outline writer attempted to rewrite original object {ref.idnum}')
            generation=root.generation if ref.idnum==root.idnum else 0
            offset=stream.tell()
            if offset>=10**10:raise ValueError('PDF exceeds classic cross-reference offset limit')
            positions[ref.idnum]=(offset,generation)
            stream.write(f'{ref.idnum} {generation} obj\n'.encode('ascii'))
            self.get_object(ref).write_to_stream(stream)
            stream.write(b'\nendobj\n')
        xref_offset=stream.tell();stream.write(b'xref\n')
        blocks=[]
        for ident in sorted(positions):
            if not blocks or ident!=blocks[-1][-1]+1:blocks.append([])
            blocks[-1].append(ident)
        for block in blocks:
            stream.write(f'{block[0]} {len(block)}\n'.encode('ascii'))
            for ident in block:
                offset,generation=positions[ident]
                stream.write(f'{offset:010d} {generation:05d} n \n'.encode('ascii'))
        trailer=DictionaryObject({NameObject('/Size'):NumberObject(len(self._objects)+1),NameObject('/Root'):IndirectObject(root.idnum,root.generation,self),NameObject('/Prev'):NumberObject(self._reader._startxref)})
        for key in ('/Info','/ID'):
            if key in self._reader.trailer:trailer[NameObject(key)]=self._reader.trailer.raw_get(key)
        stream.write(b'trailer\n');trailer.write_to_stream(stream)
        stream.write(f'\nstartxref\n{xref_offset}\n%%EOF\n'.encode('ascii'))
