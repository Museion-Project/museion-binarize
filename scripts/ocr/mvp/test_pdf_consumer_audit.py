import tempfile
import unittest
from pathlib import Path
import fitz
from .pdf_consumer_audit import correspondence, poppler_nodes, glyph_tokens, source_order_check, audit


class ConsumerAuditTests(unittest.TestCase):
    def member(self, identity, text, box):
        return dict(id=identity, text=text, bbox=box)

    def test_aggregate_keeps_offsets_without_fabricated_positions(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp)/'bbox.xml'
            path.write_text('<html><page><flow><block><line><word xMin="0" yMin="0" xMax="90" yMax="10">qualify the</word></line></block></flow></page></html>')
            node = poppler_nodes(path)[0][0]
            self.assertEqual([t['offset'] for t in node['tokens']], [[0,7],[8,11]])
            self.assertTrue(all(t['bbox'] is None for t in node['tokens']))
            result = correspondence([self.member('a','qualify',[0,0,60,10]), self.member('b','the',[60,0,90,10])],node['tokens'])
            self.assertEqual(result['missing'], [])
            self.assertFalse(result['complete_positions'])

    def test_one_hit_cannot_certify_two_repeated_members(self):
        source = [self.member('a','the',[0,0,30,10]),self.member('b','the',[0,0,30,10])]
        result = correspondence(source,[dict(text='the',bbox=[0,0,30,10])])
        self.assertEqual(len(result['unmatched_source']),1)
        self.assertFalse(result['complete_positions'])

    def test_repeated_words_resolve_by_independent_geometry(self):
        source = [self.member('a','the',[0,0,30,10]),self.member('b','the',[40,0,70,10])]
        result = correspondence(source,[dict(text='the',bbox=[40,0,70,10]),dict(text='the',bbox=[0,0,30,10])])
        self.assertTrue(result['complete_positions'])
        self.assertEqual([m['consumer_token'] for m in result['members']],[1,0])

    def test_ambiguous_bijection_stays_unknown(self):
        source = [self.member('a','the',[0,0,30,10]),self.member('b','the',[0,0,30,10])]
        result = correspondence(source,[dict(text='the',bbox=[0,0,30,10])]*2)
        self.assertTrue(all(m['state']=='UNKNOWN_AMBIGUOUS' for m in result['members']))

    def test_substring_fusion_and_combining_marks_are_literal(self):
        source=[self.member('a','qualify',[0,0,30,10]),self.member('b','the',[30,0,60,10])]
        result=correspondence(source,[dict(text='qualifythe',bbox=[0,0,60,10])])
        self.assertEqual(result['missing'],['qualify','the'])
        self.assertEqual(result['unexpected'],['qualifythe'])
        result=correspondence([self.member('a','α\u0301',[0,0,30,10])],[dict(text='ά',bbox=[0,0,30,10])])
        self.assertFalse(result['complete_positions'])

    def test_intersection_alone_does_not_prove_position(self):
        result=correspondence([self.member('a','Steel,',[0,0,30,10])],[dict(text='Steel,',bbox=[29,0,100,10])])
        self.assertEqual(result['unmatched_source'],['a'])

    def test_real_glyph_boxes_and_punctuation(self):
        doc=fitz.open();page=doc.new_page()
        page.insert_text((20,40),'Steel, qualify the =')
        tokens=glyph_tokens(page)
        self.assertEqual([t['text'] for t in tokens],['Steel,','qualify','the','='])
        self.assertTrue(all(t['bbox'][2]>t['bbox'][0] for t in tokens))
        doc.close()

    def test_internal_sort_is_not_source_order_proof(self):
        self.assertEqual(source_order_check(None,[],{},'x')['state'],'INSUFFICIENT')

    def test_cross_line_column_and_small_member_do_not_borrow_hit(self):
        source=[self.member('body','the',[0,0,30,10]),self.member('margin','the',[100,0,130,10]),
                self.member('foot','the',[0,50,30,60]),self.member('sup','2',[32,0,36,5])]
        result=correspondence(source,[dict(text='the',bbox=[0,0,30,10]),dict(text='2',bbox=[32,10,36,15])])
        self.assertFalse(result['complete_positions'])
        self.assertEqual(result['unmatched_source'],['margin','foot','sup'])

    def test_rotation_uses_actual_display_coordinates(self):
        doc=fitz.open();page=doc.new_page(width=200,height=100);page.insert_text((10,20),'word');page.set_rotation(90)
        token=glyph_tokens(page)[0]
        self.assertGreater(token['bbox'][0],50)
        self.assertEqual(token['text'],'word');doc.close()

    def test_real_export_replay_verifies_artifacts_and_unknown_source_order(self):
        import json
        from . import store,core
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);source=root/'source.pdf';doc=fitz.open();doc.new_page(width=200,height=100);doc.save(source);doc.close()
            word=dict(id='a',text='word',bbox=[10,10,45,25],engine='apple',line_id='1',confidence=1)
            snapshot=dict(schema_version=1,revision=0,source_pdf=str(source),input_sha256=core.sha(source),receipts=[],
                          pages=[dict(page=1,route='ocr',status='OCR_DRAFT',width=200,height=100,words=[word],original_apple=[word.copy()])])
            folder=store.publish(root/'out',snapshot)
            with fitz.open(folder/'searchable.pdf') as pdf:
                token=glyph_tokens(pdf[0])[0];x,y,r,b=token['bbox']
            xml=root/'bbox.xml';xml.write_text(f'<html><page><word xMin="{x}" yMin="{y}" xMax="{r}" yMax="{b}">word</word></page></html>')
            raw=root/'raw.txt';raw.write_text('word\f')
            result=audit(folder/'snapshot.json',folder/'searchable.pdf',xml,raw)
            self.assertTrue(result['pages'][0]['pixels_equal'])
            self.assertEqual(result['state'],'INSUFFICIENT')
            self.assertEqual(result['failures'],[])
            (folder/'pages.json').write_text('corrupted')
            with self.assertRaisesRegex(ValueError,'CONSUMER_ARTIFACT_CORRUPT'):
                audit(folder/'snapshot.json',folder/'searchable.pdf',xml,raw)


if __name__ == '__main__':
    unittest.main()
