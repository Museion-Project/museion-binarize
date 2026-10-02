import tempfile,unittest,unicodedata
from pathlib import Path
import fitz
import pipeline,search

class SearchTests(unittest.TestCase):
    def test_canonical_equivalence_without_accent_loss(self):
        with tempfile.TemporaryDirectory()as folder:
            d=fitz.open();p=d.new_page();pipeline.insert_hidden(p,[dict(text='τίνος',bbox=[20,20,100,40])],p.rect.width,p.rect.height)
            path=Path(folder)/'greek.pdf';d.save(path)
            self.assertTrue(search.search(path,'τίνος')['hits'])
            self.assertTrue(search.search(path,unicodedata.normalize('NFD','τίνος'))['hits'])
            self.assertFalse(search.search(path,'τινος')['hits'])
            self.assertEqual(search.search(path,'τίνος')['pdf_query'],'τίνος')

if __name__=='__main__':unittest.main()
