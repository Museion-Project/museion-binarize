import copy,json,re,shutil,subprocess,tempfile,unittest
from pathlib import Path
import fitz
import pipeline as p

class FreeOcrTests(unittest.TestCase):
    def word(self,id,text,box,engine='apple',confidence=95,line='a'):
        return dict(id=id,text=text,bbox=box,engine=engine,confidence=confidence,line_id=line)

    def test_real_greek_replaces_derived_view_only(self):
        raw=[self.word('a','voûs',[0,0,40,10])];saved=copy.deepcopy(raw)
        words,decisions=p.compose(raw,[self.word('t','νοῦς',[0,0,40,10],'tesseract')])
        self.assertEqual(raw,saved);self.assertEqual([w['text']for w in words],['νοῦς'])
        self.assertEqual(decisions[0]['source_members'],['a'])

    def test_no_unproven_whole_line_erasure(self):
        raw=[self.word('a','Long Latin line',[0,0,200,10])]
        words,decisions=p.compose(raw,[self.word('t','νοῦς',[0,0,30,10],'tesseract')])
        self.assertEqual([w['text']for w in words],['Long Latin line']);self.assertEqual(decisions[0]['state'],'REVIEW')

    def test_partial_overlap_latin_not_duplicated(self):
        raw=[self.word('a','word',[0,0,40,10])]
        words,_=p.compose(raw,[self.word('t','word',[35,0,80,10],'tesseract')])
        self.assertEqual(len(words),1)

    def test_missing_word_uses_real_reader_geometry(self):
        words,decisions=p.compose([], [self.word('t','Timaios',[0,0,40,10],'tesseract')])
        self.assertEqual(words[0]['bbox'],[0,0,40,10]);self.assertEqual(decisions[0]['state'],'RESIDUAL_DRAFT')

    def test_pdf_unicode_spacing_geometry_and_visibility(self):
        d=fitz.open();page=d.new_page(width=400,height=200)
        before=page.get_pixmap().samples
        words=[self.word('a','Greek',[10,20,60,40]),self.word('b','νοῦς',[65,20,110,40]),self.word('c','κόσμος',[115,20,180,40])]
        p.insert_hidden(page,words,400,200)
        with tempfile.TemporaryDirectory()as temp:
            path=Path(temp)/'out.pdf';d.save(path);r=fitz.open(path)
            self.assertEqual([w[4]for w in r[0].get_text('words')],['Greek','νοῦς','κόσμος'])
            self.assertTrue(r[0].search_for('νοῦς'));self.assertEqual(r[0].get_pixmap().samples,before)
            self.assertIsNone(p.native_text(r[0])) # Hidden OCR is not certified native text.

    def test_visible_native_text_candidate(self):
        d=fitz.open();page=d.new_page();page.insert_text((20,40),'This original native text is preserved exactly and avoids unnecessary local OCR. '*2,fontsize=6)
        self.assertIsNotNone(p.native_text(page))

    def test_scan_with_native_caption_still_requires_ocr(self):
        d=fitz.open();page=d.new_page()
        page.insert_text((20,40),'Native caption and metadata do not prove the whole scanned body is searchable. '*2,fontsize=6)
        image=fitz.Pixmap(fitz.csRGB,fitz.IRect(0,0,100,100),False);image.clear_with(255)
        page.insert_image(fitz.Rect(0,60,page.rect.width,page.rect.height),pixmap=image)
        self.assertIsNone(p.native_text(page))

    def test_existing_output_is_never_overwritten(self):
        with tempfile.TemporaryDirectory()as temp:
            with self.assertRaises(FileExistsError):p.run('/does/not/exist',temp,'/bad/helper')

    def test_uncertain_hidden_layer_does_not_receive_duplicate_ocr(self):
        with tempfile.TemporaryDirectory()as folder:
            source=Path(folder)/'in.pdf';out=Path(folder)/'result'
            d=fitz.open();page=d.new_page()
            p.insert_hidden(page,[dict(text='Existing OCR',bbox=[20,20,100,40])],page.rect.width,page.rect.height)
            d.save(source)
            rows=p.run(source,out,Path(__file__))
            self.assertEqual(rows[0]['status'],'EXISTING_TEXT_REVIEW')
            reopened=fitz.open(out/'searchable.pdf')
            self.assertEqual(reopened[0].get_text().count('Existing'),1)
            self.assertIn('Existing OCR',(out/'text.txt').read_text())
            self.assertIn('Existing OCR',(out/'review.html').read_text())

    def test_rotated_scan_is_explicitly_preserved_for_review(self):
        with tempfile.TemporaryDirectory()as folder:
            source=Path(folder)/'in.pdf';out=Path(folder)/'result'
            d=fitz.open();page=d.new_page();page.set_rotation(90);d.save(source)
            rows=p.run(source,out,Path(__file__))
            self.assertEqual(rows[0]['status'],'ROTATED_REVIEW')
            reopened=fitz.open(out/'searchable.pdf')
            self.assertEqual(reopened[0].rotation,90)

    def test_html_escapes_ocr_content(self):
        out=p.review_html([dict(page=1,status='DRAFT',route='native',native_text='<script>alert(1)</script>',image='x.png')])
        self.assertIn('&lt;script&gt;',out)

    @unittest.skipUnless(shutil.which('node'),'Node required for JS consumer check')
    def test_review_download_javascript_executes_with_unicode_text(self):
        markup=p.review_html([dict(page=1,status='DRAFT',route='native',native_text='νοῦς',image='x.png')])
        script=re.findall(r'<script>(.*?)</script>',markup,re.S)[0]
        fixture="""let saved='',clicked=false;
global.document={querySelectorAll:()=>[{value:'νοῦς'}],createElement:()=>({click:()=>clicked=true})};
global.Blob=class {constructor(parts){saved=parts.join('')}};
global.URL={createObjectURL:()=> 'local-test',revokeObjectURL:()=>{}};
"""
        result=subprocess.run(['node','-e',fixture+script+";download();if(saved!=='第 1 页\\nνοῦς'||!clicked)process.exit(2);"],capture_output=True,text=True)
        self.assertEqual(result.returncode,0,result.stderr)

    def test_malformed_pdf_is_explicitly_rejected_without_output(self):
        with tempfile.TemporaryDirectory()as folder:
            source=Path(folder)/'bad.pdf';source.write_bytes(b'not a PDF');out=Path(folder)/'result'
            with self.assertRaises(fitz.FileDataError):p.run(source,out,Path(__file__))
            self.assertFalse(out.exists())

    def test_blank_page_has_explicit_empty_status(self):
        with tempfile.TemporaryDirectory()as folder:
            source=Path(folder)/'blank.pdf';out=Path(folder)/'result'
            d=fitz.open();d.new_page();d.save(source)
            helper=Path(folder)/'fixture-helper'
            helper.write_text('#!/usr/bin/env python3\nimport json,pathlib,sys\np=pathlib.Path(sys.argv[2]);p.mkdir();(p/"page.json").write_text(json.dumps({"state":"locally_recognized","observations":[]}))\n')
            helper.chmod(0o700)
            rows=p.run(source,out,helper)
            self.assertEqual(rows[0]['status'],'EMPTY')
            self.assertEqual(len(fitz.open(out/'searchable.pdf')),1)

if __name__=='__main__':unittest.main()
