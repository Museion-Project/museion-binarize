import tempfile
import unittest
import copy
import json
from pathlib import Path
import fitz
from .pdf_consumer_audit import correspondence, poppler_nodes, glyph_tokens, source_order_check, source_geometry_check, project_members, sha, audit


def multipart_review_fixture(root):
    """Controlled printed template, frozen before any candidate PDF exists.

    XML below is a synthetic consumer fixture, not an invocation of Poppler.
    No OCR, research book, human review or product admission is exercised.
    """
    from . import store,core
    from scripts.ocr.free_local.pipeline import FONT
    root=Path(root);source=root/'source.pdf';literal='same same'
    with fitz.open() as template:
        page=template.new_page(width=200,height=100)
        writer=fitz.TextWriter(page.rect)
        writer.append((20,40),literal,font=fitz.Font(fontfile=str(FONT)),fontsize=12)
        writer.write_text(page)
        source_tokens=glyph_tokens(page)
        pixels=page.get_pixmap(dpi=144,alpha=False)
        with fitz.open() as image_doc:
            scan=image_doc.new_page(width=200,height=100)
            scan.insert_image(scan.rect,pixmap=pixels)
            image_doc.save(source)
    source_template=root/'source-template.json'
    source_template.write_text(json.dumps(dict(literal=literal,tokens=source_tokens,
                                               basis='controlled printed template before candidate export')))
    box=fitz.Rect(source_tokens[0]['bbox'])|fitz.Rect(source_tokens[1]['bbox'])
    word=dict(id='parent',text='samesame',bbox=list(box),engine='apple',line_id='1',
              confidence=1,source_members=['raw-a'])
    original=dict(word,id='raw-a',source_members=['raw-a'])
    out=root/'out';raw=out/'raw/page.json';raw.parent.mkdir(parents=True)
    raw.write_text(json.dumps(original))
    snapshot=dict(schema_version=1,revision=0,source_pdf=str(source),input_sha256=sha(source),receipts=[],
                  font_path=str(FONT),fallback_font_paths=[],
                  pages=[dict(page=1,route='ocr',status='OCR_DRAFT',width=200,height=100,
                              words=[word],original_apple=[original],contributions=core.ownership([word]),
                              raw_files={'raw/page.json':sha(raw)})])
    old=store.publish(out,snapshot)
    protected={str(p):sha(p) for p in [source,source_template,raw,*old.iterdir()]}
    receipt=store.review_save(out,dict(expected_revision=0,input_sha256=sha(source),
                             actions=[dict(page=1,member_id='parent',action='change',text=literal)]))
    loaded,folder=store.load_snapshot(out)
    parent=dict(id='parent',text=literal,bbox=list(box));leaves=project_members([parent])
    image=root/'source.png'
    with fitz.open(source) as doc:doc[0].get_pixmap(dpi=144,alpha=False).save(image)
    review=dict(schema='source-member-geometry-review/2',basis='source-pixel-review',
                source_sha256=sha(source),pdf_sha256=sha(folder/'searchable.pdf'),physical_page=1,
                reviewer='controlled-template-test',review_kind='AI_SOURCE_REVIEW',
                source_image=dict(path=str(image),sha256=sha(image),dpi=144),
                members=[dict(member_id='parent',export_literal=literal,state='SOURCE_POSITION_REVIEWED',
                              tokens=[dict(token_id=leaf['id'],offset=leaf['literal_offset'],text=leaf['text'],
                                           state='SOURCE_POSITION_REVIEWED',bbox=token['bbox'],
                                           position_basis='source-pixels',note='controlled print template frozen before export')
                                      for leaf,token in zip(leaves,source_tokens)])])
    review_path=root/'review.json';review_path.write_text(json.dumps(review))
    row_box=list(box+(-3,-3,3,3))
    ledger=dict(consumer_ready=True,source_sha256=sha(source),pdf_sha256=sha(folder/'searchable.pdf'),
                basis='source-pixels',evidence_path=str(source_template),evidence_sha256=sha(source_template),
                rows=[dict(row_id='r',role='body',column_id='c',bbox=row_box,
                           member_ids=[t['id'] for t in leaves])],
                member_review=dict(path=str(review_path),sha256=sha(review_path)))
    with fitz.open(folder/'searchable.pdf') as doc:actual=glyph_tokens(doc[0])
    xml=root/'synthetic-bbox.xml'
    xml.write_text('<html><page>'+''.join(
        f'<word xMin="{t["bbox"][0]}" yMin="{t["bbox"][1]}" xMax="{t["bbox"][2]}" yMax="{t["bbox"][3]}">{t["text"]}</word>'
        for t in actual)+'</page></html>')
    raw_text=root/'synthetic-raw.txt';raw_text.write_text(literal+'\f')
    return dict(source=source,out=out,old=old,folder=folder,loaded=loaded,receipt=receipt,
                protected=protected,parent=parent,leaves=leaves,review=review,review_path=review_path,
                ledger=ledger,xml=xml,raw_text=raw_text,actual=actual)


class ConsumerAuditTests(unittest.TestCase):
    def member(self, identity, text, box):
        return dict(id=identity, text=text, bbox=box)

    def rewrite_review(self,fixture):
        fixture['review_path'].write_text(json.dumps(fixture['review']))
        fixture['ledger']['member_review']['sha256']=sha(fixture['review_path'])

    def check_fixture_geometry(self,fixture):
        with fitz.open(fixture['source']) as doc:
            return source_geometry_check(fixture['ledger'],[fixture['parent']],
                                         dict(input_sha256=sha(fixture['source'])),
                                         sha(fixture['folder']/'searchable.pdf'),1,doc[0])

    def test_multipart_projection_is_literal_immutable_and_has_no_guessed_child_boxes(self):
        literal='α\u0301 😀 “same,” same ='
        parents=[self.member('p',literal,[0,0,100,10])];before=copy.deepcopy(parents)
        tokens=project_members(parents)
        self.assertEqual(parents,before)
        self.assertEqual([t['literal_offset'] for t in tokens],[[0,2],[3,4],[5,12],[13,17],[18,19]])
        self.assertEqual([t['text'] for t in tokens],['α\u0301','😀','“same,”','same','='])
        self.assertTrue(all(t['bbox'] is None for t in tokens))
        self.assertEqual(project_members(parents),tokens)
        self.assertNotEqual(project_members([dict(parents[0],text=literal.replace('α\u0301','ά'))])[0]['id'],tokens[0]['id'])
        with self.assertRaisesRegex(ValueError,'DUPLICATE_SOURCE_MEMBER'):project_members(parents*2)
        with self.assertRaisesRegex(ValueError,'EMPTY_SOURCE_MEMBER'):project_members([dict(parents[0],text='  ')])

    def test_parent_aggregate_cannot_prove_child_positions_or_consume_one_hit_twice(self):
        leaves=project_members([self.member('p','same same',[0,0,100,10])])
        result=correspondence(leaves,[dict(text='same',bbox=[0,0,30,10])])
        self.assertEqual(len(result['unmatched_source']),1);self.assertFalse(result['complete_positions'])
        result=correspondence(leaves,[dict(text='same',bbox=[0,0,30,10]),dict(text='same',bbox=[60,0,90,10])])
        self.assertTrue(all(t['state']=='UNKNOWN_AMBIGUOUS' for t in result['members']))
        distinct=project_members([self.member('p','qualify the',[0,0,100,10])])
        result=correspondence(distinct,[dict(text='qualify',bbox=[0,0,60,10]),dict(text='the',bbox=[70,0,90,10])])
        self.assertTrue(all(t['state']=='TEXT_ONLY_SOURCE_AGGREGATE' for t in result['members']))

    def test_controlled_multipart_review_save_reload_keeps_parent_raw_and_prior_revision(self):
        with tempfile.TemporaryDirectory() as temp:
            f=multipart_review_fixture(Path(temp));loaded=f['loaded'];word=loaded['pages'][0]['words'][0]
            prior=json.loads((f['old']/'snapshot.json').read_text())
            self.assertEqual(loaded['revision'],1);self.assertEqual(word['text'],'same same')
            for key in ('id','bbox','source_members','engine'):
                self.assertEqual(word[key],prior['pages'][0]['words'][0][key])
            for key in ('original_apple','contributions','raw_files'):
                self.assertEqual(loaded['pages'][0][key],prior['pages'][0][key])
            self.assertTrue(all(sha(path)==h for path,h in f['protected'].items()))
            self.assertFalse(f['receipt']['receipt']['human_approval_claimed'])
            self.assertEqual([t['text'] for t in f['actual']],['same','same'])
            geometry=self.check_fixture_geometry(f)
            self.assertEqual(geometry['state'],'PASS')
            self.assertTrue(correspondence(geometry['members'],list(reversed(f['actual'])))['complete_positions'])
            result=audit(f['folder']/'snapshot.json',f['folder']/'searchable.pdf',f['xml'],f['raw_text'],{'1':f['ledger']})
            self.assertEqual(result['state'],'PASS');self.assertTrue(result['pages'][0]['pixels_equal'])
            self.assertEqual(result['pages'][0]['supported_parent_members'],1)
            self.assertEqual(result['pages'][0]['supported_words'],2)
            self.assertFalse(result['pages'][0]['mupdf']['complete_positions'])
            self.assertTrue(result['pages'][0]['independent_mupdf_positions']['complete_positions'])
            # A real aggregate XML object still cannot supply individual boxes.
            x,y,r,b=f['parent']['bbox']
            f['xml'].write_text(f'<html><page><word xMin="{x}" yMin="{y}" xMax="{r}" yMax="{b}">same same</word></page></html>')
            result=audit(f['folder']/'snapshot.json',f['folder']/'searchable.pdf',f['xml'],f['raw_text'],{'1':f['ledger']})
            self.assertEqual(result['state'],'INSUFFICIENT')
            self.assertFalse(result['pages'][0]['independent_poppler_positions']['complete_positions'])

    def test_multipart_proof_cannot_use_v1_parent_box_or_partial_token_review(self):
        with tempfile.TemporaryDirectory() as temp:
            f=multipart_review_fixture(Path(temp));record=f['review']['members'][0]
            record['tokens'][1]['state']='UNKNOWN';self.rewrite_review(f)
            result=self.check_fixture_geometry(f)
            self.assertEqual(result['state'],'INSUFFICIENT');self.assertEqual(len(result['members']),1)
            self.assertEqual(result['unresolved_member_ids'],['parent'])
            record['tokens'][1]['state']='REJECTED';self.rewrite_review(f)
            self.assertEqual(self.check_fixture_geometry(f)['state'],'FAIL')
            f['review']['schema']='source-member-geometry-review/1'
            record.update(bbox=f['parent']['bbox'],position_basis='source-pixels',note='whole parent only')
            self.rewrite_review(f)
            self.assertEqual(self.check_fixture_geometry(f)['state'],'INSUFFICIENT')

    def test_multipart_proof_binds_exact_token_coverage_offsets_and_parent_literal(self):
        with tempfile.TemporaryDirectory() as temp:
            f=multipart_review_fixture(Path(temp));original=copy.deepcopy(f['review'])
            mutations=[('SOURCE_TOKEN_REVIEW_COVERAGE',lambda r:r['tokens'].pop()),
                       ('SOURCE_TOKEN_REVIEW_COVERAGE',lambda r:r['tokens'].append(copy.deepcopy(r['tokens'][0]))),
                       ('SOURCE_TOKEN_REVIEW_LITERAL_CHANGED',lambda r:r['tokens'][0].update(offset=[1,5])),
                       ('SOURCE_TOKEN_REVIEW_LITERAL_CHANGED',lambda r:r['tokens'][0].update(offset=[False,4])),
                       ('SOURCE_TOKEN_REVIEW_LITERAL_CHANGED',lambda r:r['tokens'][0].update(text='Same')),
                       ('SOURCE_MEMBER_REVIEW_LITERAL_CHANGED',lambda r:r.update(export_literal='same  same'))]
            for message,mutate in mutations:
                with self.subTest(message=message):
                    f['review']=copy.deepcopy(original);mutate(f['review']['members'][0]);self.rewrite_review(f)
                    with self.assertRaisesRegex(ValueError,message):self.check_fixture_geometry(f)

    def test_multipart_proof_rejects_reused_overlapping_or_aggregate_boxes(self):
        with tempfile.TemporaryDirectory() as temp:
            f=multipart_review_fixture(Path(temp));original=copy.deepcopy(f['review'])
            for box in (original['members'][0]['tokens'][0]['bbox'],f['parent']['bbox']):
                f['review']=copy.deepcopy(original);f['review']['members'][0]['tokens'][1]['bbox']=box
                self.rewrite_review(f)
                with self.assertRaisesRegex(ValueError,'SOURCE_TOKEN_GEOMETRY_OVERLAP_OR_AGGREGATE'):
                    self.check_fixture_geometry(f)

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

    def test_unaccepted_source_row_proposal_cannot_pass_by_member_order(self):
        proposal=dict(consumer_ready=False,rows=[dict(member_ids=['a'])])
        self.assertEqual(source_order_check(proposal,[self.member('a','word',[0,0,40,10])],{},'x')['state'],'INSUFFICIENT')

    def source_review_fixture(self,root,page):
        import json
        image=root/'source.png';page.get_pixmap(dpi=144,alpha=False).save(image)
        review=dict(schema='source-member-geometry-review/1',basis='source-pixel-review',
                    source_sha256='s',pdf_sha256='p',physical_page=1,reviewer='synthetic-test',
                    review_kind='AI_SOURCE_REVIEW',source_image=dict(path=str(image),sha256=sha(image),dpi=144),
                    members=[dict(member_id='a',export_literal='word',source_literal='word',
                                  state='SOURCE_POSITION_REVIEWED',bbox=[10,10,45,25],
                                  position_basis='source-pixels',note='synthetic measured source position')])
        path=root/'review.json';path.write_text(json.dumps(review))
        ledger=dict(consumer_ready=True,member_review=dict(path=str(path),sha256=sha(path)))
        return ledger,review,path

    def test_source_geometry_is_bound_to_actual_pixels_page_literal_and_multiplicity(self):
        import json
        with tempfile.TemporaryDirectory() as directory,fitz.open() as doc:
            root=Path(directory);page=doc.new_page(width=100,height=60);page.insert_text((10,22),'word')
            ledger,review,path=self.source_review_fixture(root,page)
            source=[self.member('a','word',[0,0,100,60])]
            result=source_geometry_check(ledger,source,dict(input_sha256='s'),'p',1,page)
            self.assertEqual(result['state'],'PASS');self.assertEqual(result['members'][0]['bbox'],[10,10,45,25])
            self.assertFalse(result['recognition_quality_verified']);self.assertFalse(result['human_checked'])
            review['members'].append(dict(review['members'][0]));path.write_text(json.dumps(review));ledger['member_review']['sha256']=sha(path)
            with self.assertRaisesRegex(ValueError,'SOURCE_MEMBER_REVIEW_COVERAGE'):
                source_geometry_check(ledger,source,dict(input_sha256='s'),'p',1,page)

    def test_source_review_rejects_wrong_page_and_keeps_blank_or_pending_member_unknown(self):
        import json
        with tempfile.TemporaryDirectory() as directory,fitz.open() as doc:
            root=Path(directory);page=doc.new_page(width=100,height=60);page.insert_text((10,22),'word')
            ledger,review,path=self.source_review_fixture(root,page);source=[self.member('a','word',[0,0,100,60])]
            other=doc.new_page(width=100,height=60)
            with self.assertRaisesRegex(ValueError,'SOURCE_MEMBER_IMAGE_PAGE_MISMATCH'):
                source_geometry_check(ledger,source,dict(input_sha256='s'),'p',1,other)
            page=doc[0];review['members'][0]['bbox']=[60,40,90,55]
            path.write_text(json.dumps(review));ledger['member_review']['sha256']=sha(path)
            self.assertEqual(source_geometry_check(ledger,source,dict(input_sha256='s'),'p',1,page)['state'],'INSUFFICIENT')
            review['members'][0]['state']='UNKNOWN';path.write_text(json.dumps(review));ledger['member_review']['sha256']=sha(path)
            self.assertEqual(source_geometry_check(ledger,source,dict(input_sha256='s'),'p',1,page)['unresolved_member_ids'],['a'])

    def test_source_geometry_cannot_reuse_one_support_for_distinct_parents(self):
        cases=[('copied box',[10,10,45,25],None,'SOURCE_TOKEN_GEOMETRY_REUSED'),
               ('same source pixels',[10.01,10.01,44.99,24.99],None,'SOURCE_TOKEN_GEOMETRY_REUSED'),
               ('same source cell at different boxes',[50,10,85,25],'source-cell-1','SOURCE_TOKEN_SOURCE_CELL_REUSED')]
        for label,second_box,cell,error in cases:
            with self.subTest(label=label),tempfile.TemporaryDirectory() as directory,fitz.open() as doc:
                root=Path(directory);page=doc.new_page(width=100,height=60)
                page.insert_text((10,22),'word');page.insert_text((50,22),'word')
                ledger,review,path=self.source_review_fixture(root,page)
                second=dict(review['members'][0],member_id='b',bbox=second_box)
                if cell:
                    review['members'][0]['source_cell_id']=cell;second['source_cell_id']=cell
                review['members'].append(second);path.write_text(json.dumps(review));ledger['member_review']['sha256']=sha(path)
                source=[self.member(mid,'word',[0,0,100,60])for mid in ['a','b']]
                with self.assertRaisesRegex(ValueError,error):
                    source_geometry_check(ledger,source,dict(input_sha256='s'),'p',1,page)

    def test_source_cell_ownership_covers_multipart_leaves_and_other_parents(self):
        with tempfile.TemporaryDirectory() as directory,fitz.open() as doc:
            root=Path(directory);page=doc.new_page(width=100,height=60)
            page.insert_text((10,22),'word');page.insert_text((50,22),'next');page.insert_text((10,42),'word')
            ledger,review,path=self.source_review_fixture(root,page)
            source=[self.member('a','word next',[0,0,100,60]),self.member('b','word',[0,0,100,60])]
            leaves=project_members(source);boxes=[[10,10,45,25],[50,10,85,25],[10,30,45,45]]
            proofs=[dict(token_id=t['id'],offset=t['literal_offset'],text=t['text'],state='SOURCE_POSITION_REVIEWED',
                         bbox=b,position_basis='source-pixels',note='independent synthetic source support',source_cell_id=c)
                    for t,b,c in zip(leaves,boxes,['cell-a','cell-next','cell-a'])]
            review['schema']='source-member-geometry-review/2'
            review['members']=[dict(member_id=s['id'],export_literal=s['text'],state='SOURCE_POSITION_REVIEWED',
                                    tokens=[p for p,t in zip(proofs,leaves)if t['parent_member_id']==s['id']])for s in source]
            path.write_text(json.dumps(review));ledger['member_review']['sha256']=sha(path)
            with self.assertRaisesRegex(ValueError,'SOURCE_TOKEN_SOURCE_CELL_REUSED'):
                source_geometry_check(ledger,source,dict(input_sha256='s'),'p',1,page)
            review['members'][1]['tokens'][0]['source_cell_id']='cell-b'
            path.write_text(json.dumps(review));ledger['member_review']['sha256']=sha(path)
            result=source_geometry_check(ledger,source,dict(input_sha256='s'),'p',1,page)
            self.assertEqual(result['state'],'PASS');self.assertEqual(len(result['members']),3)
            self.assertEqual([m['source_cell_id']for m in result['members']],['cell-a','cell-next','cell-b'])

    def test_distinct_repeated_words_may_have_overlapping_source_regions(self):
        with tempfile.TemporaryDirectory() as directory,fitz.open() as doc:
            root=Path(directory);page=doc.new_page(width=100,height=60)
            page.insert_text((10,22),'word');page.insert_text((50,22),'word')
            ledger,review,path=self.source_review_fixture(root,page)
            review['members'][0].update(bbox=[10,10,51,25],source_cell_id='cell-a')
            review['members'].append(dict(review['members'][0],member_id='b',bbox=[50,10,85,25],source_cell_id='cell-b'))
            path.write_text(json.dumps(review));ledger['member_review']['sha256']=sha(path)
            source=[self.member(mid,'word',[0,0,100,60])for mid in ['a','b']]
            result=source_geometry_check(ledger,source,dict(input_sha256='s'),'p',1,page)
            self.assertEqual(result['state'],'PASS');self.assertEqual([m['id']for m in result['members']],['a','b'])
            self.assertEqual([m['source_cell_id']for m in result['members']],['cell-a','cell-b'])
            self.assertFalse(result['recognition_quality_verified']);self.assertFalse(result['human_checked'])

    def overlapping_multipart_source_fixture(self, root, page):
        """The same independent source regions as the separate-parent control."""
        page.insert_text((10,22),'word');page.insert_text((50,22),'word')
        ledger,review,path=self.source_review_fixture(root,page)
        source=[self.member('parent','word word',[0,0,100,60])]
        leaves=project_members(source)
        proofs=[dict(token_id=t['id'],offset=t['literal_offset'],text=t['text'],
                     state='SOURCE_POSITION_REVIEWED',bbox=box,source_cell_id=cell,
                     position_basis='source-pixels',note='independently placed synthetic word')
                for t,box,cell in zip(leaves,[[10,10,51,25],[50,10,85,25]],['cell-a','cell-b'])]
        review['schema']='source-member-geometry-review/2'
        review['members']=[dict(member_id='parent',export_literal='word word',
                                state='SOURCE_POSITION_REVIEWED',tokens=proofs)]
        return ledger,review,path,source

    def test_independent_source_region_acceptance_is_invariant_to_parent_grouping(self):
        with tempfile.TemporaryDirectory() as directory,fitz.open() as doc:
            root=Path(directory);page=doc.new_page(width=100,height=60)
            ledger,review,path,source=self.overlapping_multipart_source_fixture(root,page)
            multipart=copy.deepcopy(review)
            review['schema']='source-member-geometry-review/1'
            review['members']=[dict(p,member_id=mid,export_literal='word')
                               for p,mid in zip(multipart['members'][0]['tokens'],['a','b'])]
            path.write_text(json.dumps(review));ledger['member_review']['sha256']=sha(path)
            separate=source_geometry_check(ledger,[self.member(mid,'word',[0,0,100,60]) for mid in ['a','b']],
                                           dict(input_sha256='s'),'p',1,page)
            self.assertEqual(separate['state'],'PASS')
            path.write_text(json.dumps(multipart));ledger['member_review']['sha256']=sha(path)
            before=copy.deepcopy(source)
            grouped=source_geometry_check(ledger,source,dict(input_sha256='s'),'p',1,page)
            self.assertEqual(grouped['state'],'PASS');self.assertEqual(source,before)
            self.assertEqual([m['bbox'] for m in grouped['members']],[m['bbox'] for m in separate['members']])
            self.assertEqual([m['source_cell_id'] for m in grouped['members']],['cell-a','cell-b'])
            self.assertEqual([m['literal_offset'] for m in grouped['members']],[[0,4],[5,9]])
            self.assertFalse(grouped['recognition_quality_verified']);self.assertFalse(grouped['human_checked'])

    def test_overlapping_multipart_regions_cannot_reuse_pixels_or_source_cells(self):
        for collision,error in [('pixels','SOURCE_TOKEN_GEOMETRY_REUSED'),
                                ('cell','SOURCE_TOKEN_SOURCE_CELL_REUSED')]:
            with self.subTest(collision=collision),tempfile.TemporaryDirectory() as directory,fitz.open() as doc:
                root=Path(directory);page=doc.new_page(width=100,height=60)
                ledger,review,path,source=self.overlapping_multipart_source_fixture(root,page)
                first,second=review['members'][0]['tokens']
                if collision=='pixels':second['bbox']=[10.01,10.01,50.99,24.99]
                else:second['source_cell_id']=first['source_cell_id']
                path.write_text(json.dumps(review));ledger['member_review']['sha256']=sha(path)
                with self.assertRaisesRegex(ValueError,error):
                    source_geometry_check(ledger,source,dict(input_sha256='s'),'p',1,page)

    def test_overlapping_multipart_regions_keep_unreviewed_child_pending(self):
        with tempfile.TemporaryDirectory() as directory,fitz.open() as doc:
            root=Path(directory);page=doc.new_page(width=100,height=60)
            ledger,review,path,source=self.overlapping_multipart_source_fixture(root,page)
            review['members'][0]['tokens'][1]['state']='UNKNOWN'
            path.write_text(json.dumps(review));ledger['member_review']['sha256']=sha(path)
            result=source_geometry_check(ledger,source,dict(input_sha256='s'),'p',1,page)
            self.assertEqual(result['state'],'INSUFFICIENT');self.assertEqual(len(result['members']),1)
            self.assertEqual(result['unresolved_member_ids'],['parent'])
            self.assertEqual(result['unresolved_token_ids'],[project_members(source)[1]['id']])

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
