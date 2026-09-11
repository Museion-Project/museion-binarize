import tempfile, unittest, subprocess
from pathlib import Path
from pypdf import PdfReader, PdfWriter
from pypdf.generic import RectangleObject, NameObject, DictionaryObject, ArrayObject, NumberObject
from pdf_test_fixture import make_pdf
from pdf_backend import Document
from bookmarks import export, sha

class Geometry(unittest.TestCase):
    def test_inline_signature_and_encrypted_sources_refuse_without_output(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);source=root/'base.pdf';make_pdf(source,['Protected original'])
            for kind in ('signature','encrypted'):
                writer=PdfWriter(clone_from=source)
                if kind=='signature':
                    writer.root_object[NameObject('/AcroForm')]=DictionaryObject({NameObject('/Fields'):ArrayObject([DictionaryObject({NameObject('/V'):DictionaryObject({NameObject('/ByteRange'):ArrayObject([NumberObject(v) for v in [0,1,2,3]])})})])})
                else:writer.encrypt('',owner_password='synthetic-owner')
                path=root/f'{kind}.pdf';writer.write(path);digest=sha(path)
                t=dict(schema='mpdf-bookmark-table/1',source=str(path),source_sha256=digest,page_count=1,entries=[dict(id='r',title='Root',parent=None,level=0,target_pdf_page=0,state='manually_confirmed',review_reasons=[])])
                with self.assertRaises(ValueError):export(t,root/f'{kind}-out.pdf')
                self.assertFalse((root/f'{kind}-out.pdf').exists());self.assertEqual(sha(path),digest)

    def test_compressed_objects_and_existing_incremental_revisions(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);source=root/'base.pdf';make_pdf(source,['Text and original stream'],old_outline=False)
            packed=root/'compressed.pdf'
            subprocess.run(['qpdf','--object-streams=generate',str(source),str(packed)],check=True)
            for index in range(2):
                t=dict(schema='mpdf-bookmark-table/1',source=str(packed),source_sha256=sha(packed),page_count=1,entries=[dict(id='r',title=f'Revision {index}',parent=None,level=0,target_pdf_page=0,state='manually_confirmed',review_reasons=[])])
                output=root/f'revision-{index}.pdf';export(t,output)
                subprocess.run(['qpdf','--check',str(output)],capture_output=True,check=True)
                packed=output

    def test_inherited_resources_are_preserved(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);source=root/'base.pdf';make_pdf(source,['Inherited font resource'])
            writer=PdfWriter(clone_from=source);page=writer.pages[0]
            page['/Parent'][NameObject('/Resources')]=page['/Resources']
            del page['/Resources'];path=root/'inherited.pdf';writer.write(path)
            t=dict(schema='mpdf-bookmark-table/1',source=str(path),source_sha256=sha(path),page_count=1,entries=[dict(id='r',title='Root',parent=None,level=0,target_pdf_page=0,state='manually_confirmed',review_reasons=[])])
            self.assertTrue(export(t,root/'out.pdf')['original_bytes_preserved'])
            subprocess.run(['qpdf','--check',str(root/'out.pdf')],capture_output=True,check=True)

    def test_rotated_cropped_glyph_boxes_match_rendered_ink_and_export(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);source=root/'base.pdf';make_pdf(source,['Test glyph position 123'])
            for angle in (0,90,180,270):
                path=root/f'rotated-{angle}.pdf';w=PdfWriter(clone_from=source)
                w.pages[0].cropbox=RectangleObject([30,40,550,810]);w.pages[0].rotate(angle);w.write(path)
                with Document(path) as doc:
                    page=doc[0];image=page.render();glyphs=page.glyphs()
                    self.assertEqual(image.size,tuple(round(v) for v in (page.rect.width,page.rect.height)))
                    for glyph in glyphs:
                        x,y,width,height=glyph['bbox']
                        self.assertGreaterEqual(x,0);self.assertGreaterEqual(y,0)
                        crop=image.crop((max(0,int(x)-1),max(0,int(y)-1),min(image.width,int(x+width)+2),min(image.height,int(y+height)+2)))
                        self.assertLess(min(crop.convert('L').getdata()),230,repr((angle,glyph)))
                    self.assertIn('position',' '.join(word[4] for word in page.words()))
                t=dict(schema="mpdf-bookmark-table/1",source=str(path),source_sha256=sha(path),page_count=1,entries=[dict(id='root',title='Ελληνικά 中文',parent=None,level=0,target_pdf_page=0,state='manually_confirmed',review_reasons=[])])
                receipt=export(t,root/f'export-{angle}.pdf');self.assertTrue(receipt['original_bytes_preserved'])
                subprocess.run(['qpdf','--check',str(root/f'export-{angle}.pdf')],capture_output=True,check=True)

if __name__=='__main__':unittest.main()
