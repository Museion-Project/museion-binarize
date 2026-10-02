import copy
import tempfile
import unittest
from pathlib import Path
import fitz
from scripts.ocr.mvp import core, store
from scripts.ocr.mvp.local import run_task,readiness
from scripts.ocr.free_local import pipeline as old

class MvpTests(unittest.TestCase):
    def word(self,i,t,b,engine='apple',line='1',confidence=95):return dict(id=i,text=t,bbox=b,engine=engine,line_id=line,confidence=confidence)
    def test_reading_rows_top_jitter_and_gutter(self):
        words=[self.word('b','ἕπεσθαι',[45,9,95,25]),self.word('a','χρὴ',[10,11,40,26]),
               self.word('c','other column',[200,10,270,26]),self.word('d','next',[10,40,50,55])]
        rows=core.reading_rows(words)
        self.assertEqual([[w['id'] for w in row] for row in rows],[['a','b'],['c'],['d']])
    def test_complete_punctuation_transfer_and_ambiguous_rejection(self):
        apple=self.word('a','"sample',[10,10,60,25])
        reader=self.word('t','“sample',[11,11,61,25],'tesseract')
        words,d=core.compose([apple],[reader],[])
        self.assertEqual(words[0]['text'],'“sample');self.assertEqual(words[0]['source_members'],['a'])
        self.assertEqual(d[0]['state'],'PUNCTUATION_DRAFT');core.ownership(words)
        words,_=core.compose([apple],[reader,self.word('u','sample',[40,10,60,25],'tesseract')],[])
        self.assertEqual(words[0]['text'],'"sample')
        words,_=core.compose([apple],[dict(reader,text='“simple')],[])
        self.assertEqual(words[0]['text'],'"sample')
        words,_=core.compose([apple],[dict(reader,bbox=[30,11,61,25])],[])
        self.assertEqual(words[0]['text'],'"sample')
        self.assertFalse(core.punctuation_equivalent_positions('„sample',',,sample'))
        self.assertFalse(core.punctuation_equivalent_positions('sample.','sample.”'))
        self.assertTrue(core.punctuation_equivalent_positions('"sample','“sample'))
    def test_phrase_export_common_baseline_complete_pdf_and_review(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);source=root/'source.pdf';doc=fitz.open();doc.new_page(width=300,height=200)
            doc.new_page().insert_text((20,30),'Native untouched source')
            doc.new_page().insert_text((20,30),'Selected native source')
            doc.save(source);doc.close()
            words=[self.word('b','ἕπεσθαι',[65,9,135,25]),self.word('a','χρὴ',[10,11,60,26])]
            page=dict(page=1,route='ocr',status='OCR_DRAFT',width=300,height=200,words=words,original_apple=copy.deepcopy(words))
            snapshot=dict(schema_version=1,revision=0,source_pdf=str(source),input_sha256=core.sha(source),pages=[page,dict(page=3,route='native',status='NATIVE_PRESERVED',native_text='Selected native source')],receipts=[])
            folder=store.publish(root/'out',snapshot)
            with fitz.open(folder/'searchable.pdf') as output,fitz.open(source) as original:
                self.assertEqual(len(output),3);self.assertTrue(output[0].search_for('χρὴ ἕπεσθαι'))
                self.assertEqual(len(output[0].get_text('words')),2)
                for a,b in zip(output,original):
                    self.assertEqual(a.get_pixmap().samples,b.get_pixmap().samples)
                self.assertEqual(output[1].get_text(),original[1].get_text())
                self.assertEqual(output[2].get_text(),original[2].get_text())
            store.review_save(root/'out',dict(expected_revision=0,input_sha256=snapshot['input_sha256'],actions=[dict(page=1,member_id='a',action='accept')]))
            self.assertEqual(store.load_snapshot(root/'out')[0]['consumer_policy'],core.CONSUMER_POLICY)
    def test_greek_safe_replacement_and_no_mutation(self):
        original=[self.word('a','nous',[10,10,50,22])];frozen=copy.deepcopy(original)
        words,_=core.compose(original,[self.word('t','νοῦς',[10,10,50,22],'tesseract')],[])
        self.assertEqual(original,frozen);self.assertEqual(words[0]['text'],'νοῦς')
    def test_single_greek_word_uncovered_survives(self):
        words,_=core.compose([], [self.word('t','ὁ',[10,10,20,20],'tesseract'),self.word('u','κόσμος',[22,10,60,20],'tesseract')],[])
        self.assertEqual(len(words),2)
    def test_broad_row_never_silently_erased_by_one_word(self):
        a=self.word('a','Long Latin line',[0,10,250,25])
        words,d=core.compose([a],[self.word('t','νοῦς',[10,10,45,25],'tesseract')],[])
        self.assertEqual(words[0]['id'],'a');self.assertEqual(d[0]['state'],'REVIEW')
    def test_mixed_rows_independent_words_quarantine_overlap(self):
        a=self.word('a','one two',[0,10,60,40]);b=self.word('b','two',[0,30,60,40])
        t=[self.word('t','one',[0,10,60,20],'tesseract','1'),self.word('u','two',[0,30,60,40],'tesseract','2')]
        words,d=core.compose([a,b],t,[])
        for i,w in enumerate(words):
            self.assertTrue(all(old.overlap(w['bbox'],q['bbox'])<=.6 for q in words[i+1:]))
    def test_residual_no_double_consumption(self):
        t=self.word('r','missing',[20,20,90,40],'residual-tesseract')
        words,_=core.compose([],[],[t,copy.deepcopy(t)])
        self.assertEqual(len(words),1)
    def test_residual_intersection_does_not_prove_membership(self):
        self.assertFalse(core.residual_member_in_target([5,10,80,30],[50,10,90,30]))
        self.assertTrue(core.residual_member_in_target([52,12,82,28],[50,10,90,30]))
        self.assertFalse(core.residual_member_in_target([52,31,82,49],[50,10,90,30]))
    def test_fragment_expansion_uses_source_ink_without_neighbor_row(self):
        import cv2,numpy as np
        with tempfile.TemporaryDirectory() as temp:
            image=Path(temp)/'source.png';pixels=np.full((100,200),255,np.uint8)
            pixels[30:50,40:55]=0;pixels[30:50,60:75]=0;pixels[30:50,80:95]=0
            pixels[70:90,40:95]=0;cv2.imwrite(str(image),pixels)
            region=dict(member_id='r',target_bbox=[38,29,57,51],canvas_bbox=[26,21,69,59])
            frozen=copy.deepcopy(region);expanded=core.complete_residual_support(image,[region])[0]
            self.assertEqual(region,frozen);self.assertGreaterEqual(expanded['target_bbox'][2],95)
            self.assertLess(expanded['canvas_bbox'][3],70)
    def test_reader_failure_records_wall_clock_without_ocr(self):
        import sys,json
        from scripts.ocr.mvp.local import invoke
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);stdout=root/'stdout';stderr=root/'stderr'
            with self.assertRaisesRegex(RuntimeError,'LOCAL_READER_FAILED'):
                invoke([sys.executable,'-c','raise SystemExit(3)'],2,stdout,stderr)
            record=json.loads((root/'stdout.call.json').read_text())
            self.assertEqual(record['returncode'],3);self.assertEqual(record['status'],'FAILED')
            self.assertGreater(record['wall_seconds'],0)
    def test_store_change_reload_stale_and_visible_pdf(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);source=root/'source.pdf';d=fitz.open();d.new_page(width=200,height=100);d.save(source)
            original=fitz.open(source)[0].get_pixmap().samples
            pg=dict(page=1,route='ocr',status='OCR_DRAFT',width=200,height=100,image_path=str(root/'source.png'),words=[self.word('a','νοῦς',[10,10,55,25])],original_apple=[])
            s=dict(schema_version=1,revision=0,source_pdf=str(source),input_sha256=core.sha(source),pages=[pg],receipts=[])
            out=root/'out';store.publish(out,s)
            patch=dict(expected_revision=0,input_sha256=s['input_sha256'],actions=[dict(page=1,member_id='a',action='change',text='κόσμος')])
            result=store.review_save(out,patch);reopened=fitz.open(result['artifacts']['searchable.pdf'])
            self.assertTrue(reopened[0].search_for('κόσμος'));self.assertEqual(reopened[0].get_pixmap().samples,original)
            self.assertEqual(store.load_snapshot(out)[0]['revision'],1)
            with self.assertRaisesRegex(ValueError,'STALE'):store.review_save(out,patch)
    def test_corrupted_snapshot_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);source=root/'source.pdf';d=fitz.open();d.new_page();d.save(source)
            s=dict(schema_version=1,revision=0,source_pdf=str(source),input_sha256=core.sha(source),pages=[dict(page=1,route='native',status='NATIVE_PRESERVED')],receipts=[])
            folder=store.publish(root/'out',s);(folder/'pages.json').write_text('[]')
            with self.assertRaisesRegex(ValueError,'CORRUPT'):store.load_snapshot(root/'out')
    def test_completed_legacy_resume_rejects_changed_source(self):
        # Real native PDF, completed durable artifacts and the persisted v1
        # envelope format; no mocked loader, source hash or recognition call.
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);source=root/'source.pdf';doc=fitz.open();page=doc.new_page()
            page.insert_text((20,30),'Reliable native source text for legacy completed resume. '*2,fontsize=6)
            doc.save(source);doc.close()
            task=dict(operation_id='legacy-source-guard',input_pdf=str(source),input_sha256=core.sha(source),page_numbers=[1],mode='local',output_directory=str(root/'out'),config_version=core.CONFIG_VERSION)
            # Explicit non-executable identity fixtures: native preservation
            # must never try to recognize these files or use a research helper.
            config=dict(apple_helper=str(source),tesseract=str(source))
            completed=run_task(task,config);self.assertEqual(completed['page_results'][0]['status'],'NATIVE_PRESERVED')
            job=store.read(root/'out/job.json');task=dict(job['task'],config_version='mvp-local-v1')
            job['task']=task;store.write(root/'out/job.json',job)
            self.assertEqual(run_task(task,config),completed)
            source.write_bytes(source.read_bytes()+b'\n% changed source after completion\n')
            with self.assertRaisesRegex(ValueError,'SOURCE_HASH_MISMATCH'):run_task(task,config)
    def test_native_fresh_task_keeps_source_and_explicit_mode(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);source=root/'source.pdf';d=fitz.open();p=d.new_page();p.insert_text((20,30),'This native source text avoids unnecessary OCR and remains exactly preserved. '*2,fontsize=6);d.save(source)
            task=dict(operation_id='test',input_pdf=str(source),input_sha256=core.sha(source),page_numbers=[1],mode='local',output_directory=str(root/'out'),config_version=core.CONFIG_VERSION)
            config=dict(apple_helper=str(source),tesseract=str(source))
            result=run_task(task,config);self.assertEqual(result['page_results'][0]['status'],'NATIVE_PRESERVED')
            self.assertEqual(core.sha(source),task['input_sha256']);self.assertEqual(run_task(task,config),result)
            task['mode']='';
            with self.assertRaises(ValueError):run_task(task)
    def test_complete_greek_group_owns_broad_member_once(self):
        a=self.word('a','ai',[0,10,110,35]);t=self.word('t','καὶ',[10,15,45,30],'tesseract');u=self.word('u','τὴν',[55,15,110,30],'tesseract')
        words,_=core.compose([a],[t,u],[])
        self.assertEqual([w['text'] for w in words],['καὶ','τὴν'])
        registry=core.ownership(words);self.assertEqual(len(registry),1);self.assertEqual(registry[0]['source_members'],['a'])
    def test_tall_apple_box_does_not_discard_complete_greek_word(self):
        a=self.word('a','Twv',[0,0,58,46]);t=self.word('t','των',[6,11,58,28],'tesseract')
        words,_=core.compose([a],[t],[]);self.assertEqual(words[0]['text'],'των')
    def test_source_member_cannot_be_owned_by_two_units(self):
        a=dict(self.word('t','one',[0,0,20,10],'tesseract'),source_members=['apple-a'])
        b=dict(self.word('u','two',[20,0,40,10],'tesseract'),source_members=['apple-a'])
        with self.assertRaisesRegex(ValueError,'OWNERSHIP'):core.ownership([a,b])
    def test_not_ready_without_quality_and_app(self):self.assertFalse(readiness()['ready'])

if __name__=='__main__':unittest.main()

class LifecycleTests(unittest.TestCase):
    def task(self,root,source,timeout=1):
        return dict(operation_id='lifecycle',input_pdf=str(source),input_sha256=core.sha(source),page_numbers=[1],mode='local',output_directory=str(root/'out'),config_version=core.CONFIG_VERSION)
    def test_hard_timeout_kills_child_group_and_preserves_page(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);source=root/'source.pdf';d=fitz.open();d.new_page();d.save(source)
            helper=root/'helper';helper.write_text('#!/bin/sh\nsleep 10\n');helper.chmod(0o755)
            result=run_task(self.task(root,source),dict(apple_helper=str(helper),tesseract=str(source),page_timeout_seconds=1))
            self.assertEqual(result['page_results'][0]['error'],'PAGE_TIMEOUT')
            self.assertTrue(Path(result['artifacts']['searchable_pdf']).is_file())
    def test_active_cancel_preserves_raw_and_source(self):
        import threading,time
        from scripts.ocr.mvp.local import cancel_task
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);source=root/'source.pdf';d=fitz.open();d.new_page();d.save(source)
            helper=root/'helper';helper.write_text('#!/bin/sh\nsleep 10\n');helper.chmod(0o755)
            def cancel():
                while not (root/'out/job.json').exists():time.sleep(.02)
                time.sleep(.4);cancel_task(root/'out')
            t=threading.Thread(target=cancel);t.start()
            result=run_task(self.task(root,source),dict(apple_helper=str(helper),tesseract=str(source),page_timeout_seconds=5));t.join()
            self.assertEqual(result['status'],'cancelled');self.assertEqual(result['page_results'][0]['status'],'CANCELLED')

class ExportReviewTests(unittest.TestCase):
    word=MvpTests.word
    # The shared-source case uses disjoint boxes: geometry alone cannot protect ownership.
    def test_reject_joint_preserves_other_owner_and_receipt(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);source=root/'source.pdf';doc=fitz.open();doc.new_page(width=200,height=100);doc.save(source)
            apple=self.word('a','old',[0,10,20,25])
            words=[dict(self.word('j1','one',[0,10,20,25],'tesseract'),source_members=['a'],reason='complete_row_group_independent_words'),dict(self.word('j2','two',[30,10,50,25],'tesseract'),source_members=['a'],reason='complete_row_group_independent_words')]
            page=dict(page=1,route='ocr',status='OCR_DRAFT',width=200,height=100,words=words,original_apple=[apple],contributions=core.ownership(words))
            snapshot=dict(schema_version=1,revision=0,source_pdf=str(source),input_sha256=core.sha(source),pages=[page],receipts=[])
            store.publish(root/'out',snapshot);store.review_save(root/'out',dict(expected_revision=0,input_sha256=snapshot['input_sha256'],actions=[dict(page=1,member_id='j1',action='reject')]))
            saved,_=store.load_snapshot(root/'out');self.assertEqual([w['id'] for w in saved['pages'][0]['words']],['j2']);self.assertEqual(saved['receipts'][-1]['actions'][0]['action'],'reject')
    def test_noncharacter_retained_pending_then_review_change_exported(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);source=root/'source.pdf';doc=fitz.open();doc.new_page(width=200,height=100);doc.save(source)
            text='raw\ufffe';page=dict(page=1,route='ocr',status='OCR_DRAFT',width=200,height=100,words=[self.word('a',text,[10,10,50,25])],original_apple=[])
            snapshot=dict(schema_version=1,revision=0,source_pdf=str(source),input_sha256=core.sha(source),pages=[page],receipts=[])
            folder=store.publish(root/'out',snapshot);saved,_=store.load_snapshot(root/'out')
            self.assertEqual(saved['pages'][0]['words'][0]['text'],text);self.assertEqual(saved['pages'][0]['status'],'EXPORT_REVIEW');self.assertIn(text,(folder/'text.txt').read_text());self.assertEqual(saved['pages'][0]['export_coverage']['pending_words'],1)
            receipt=store.review_save(root/'out',dict(expected_revision=0,input_sha256=snapshot['input_sha256'],actions=[dict(page=1,member_id='a',action='change',text='fixed')]))
            saved,_=store.load_snapshot(root/'out');self.assertEqual(saved['export_review'],[]);self.assertTrue(fitz.open(receipt['artifacts']['searchable.pdf'])[0].search_for('fixed'))
    def test_legal_math_fallback_or_explicit_pending_and_unknown_glyph(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);source=root/'source.pdf';doc=fitz.open();doc.new_page(width=200,height=100);doc.save(source)
            page=dict(page=1,route='ocr',status='OCR_DRAFT',width=200,height=100,words=[self.word('math','≥',[10,10,30,25]),self.word('unknown','\u0378',[40,10,60,25])],original_apple=[])
            snapshot=dict(schema_version=1,revision=0,source_pdf=str(source),input_sha256=core.sha(source),pages=[page],receipts=[])
            folder=store.publish(root/'out',snapshot);saved,_=store.load_snapshot(root/'out');math,unknown=saved['pages'][0]['words']
            self.assertEqual(unknown['export_status'],'EXPORT_REVIEW');self.assertEqual(unknown['text'],'\u0378')
            if Path('/System/Library/Fonts/Times.ttc').exists():self.assertEqual(math['export_status'],'EXPORTED');self.assertTrue(fitz.open(folder/'searchable.pdf')[0].search_for('≥'))
            else:self.assertEqual(math['export_status'],'EXPORT_REVIEW')

class FullPdfConsumerTests(unittest.TestCase):
    word=MvpTests.word
    def source(self,root):
        source=root/'source.pdf';doc=fitz.open()
        for number,(width,height) in enumerate([(220,130),(250,170),(300,190)],1):
            page=doc.new_page(width=width,height=height);page.draw_rect(fitz.Rect(100,60,150,90),color=(number/4,0,.5),fill=(0,number/4,.2));page.insert_text((10,25),f'Native page {number}')
            if number==3:page.set_rotation(90)
        doc.set_toc([[1,'First',1],[1,'Third',3]]);doc.save(source);doc.close();return source
    def snapshot(self,source,selected,failed=False):
        with fitz.open(source) as doc:
            pages=[dict(page=n,route='failed' if failed else 'ocr',status='CANCELLED' if failed else 'OCR_DRAFT',width=doc[n-1].rect.width,height=doc[n-1].rect.height,words=[] if failed else [self.word(f'w{n}',f'Added{n}',[10,35,70,50])],original_apple=[]) for n in selected]
        return dict(schema_version=1,revision=0,source_pdf=str(source),input_sha256=core.sha(source),pages=pages,receipts=[])
    def assert_preserved(self,source,pdf,selected):
        with fitz.open(source) as original,fitz.open(pdf) as exported:
            self.assertEqual(len(exported),3)
            for index in range(3):
                self.assertEqual(exported[index].rect,original[index].rect);self.assertEqual(exported[index].get_pixmap().samples,original[index].get_pixmap().samples)
                self.assertIn(f'Native page {index+1}',exported[index].get_text())
                if index+1 not in selected:self.assertEqual(exported[index].get_text(),original[index].get_text())
            self.assertEqual(exported.get_toc(),original.get_toc())
    def test_partial_out_of_order_rotation_save_reload_keeps_full_source(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);source=self.source(root);snapshot=self.snapshot(source,[3,1]);original_hash=core.sha(source);out=root/'out';folder=store.publish(out,snapshot)
            self.assert_preserved(source,folder/'searchable.pdf',[3,1]);saved,_=store.load_snapshot(out);self.assertEqual([p['page'] for p in saved['pages']],[3,1]);self.assertEqual(saved['untouched_page_numbers'],[2]);self.assertEqual(saved['pdf_page_mapping'][2]['result_index'],0);self.assertEqual(saved['source_page_count'],3)
            receipt=store.review_save(out,dict(expected_revision=0,input_sha256=original_hash,actions=[dict(page=3,member_id='w3',action='change',text='Reviewed3')]))
            self.assert_preserved(source,receipt['artifacts']['searchable.pdf'],[3,1]);self.assertTrue(fitz.open(receipt['artifacts']['searchable.pdf'])[2].search_for('Reviewed3'));self.assertEqual(core.sha(source),original_hash);self.assertEqual(store.load_snapshot(out)[0]['revision'],1)
    def test_all_selected_and_cancelled_preserve_natural_order(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);source=self.source(root)
            for name,selected,failed in [('all',[3,2,1],False),('cancelled',[3],True)]:
                snapshot=self.snapshot(source,selected,failed);folder=store.publish(root/name,snapshot);self.assert_preserved(source,folder/'searchable.pdf',[] if failed else selected)
                saved,_=store.load_snapshot(root/name);self.assertEqual(saved['exported_page_count'],3)
                if failed:self.assertEqual(saved['pages'][0]['status'],'CANCELLED')
    def test_run_native_partial_completion_mapping_without_recognition(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);source=root/'native.pdf';doc=fitz.open()
            for number in range(1,4):
                page=doc.new_page(width=300+number*10,height=200);page.insert_textbox(fitz.Rect(10,10,280,180),(f'Native source page {number} is preserved without any OCR recognition. '*4),fontsize=6)
            doc.save(source);doc.close()
            task=dict(operation_id='native-partial-map',input_pdf=str(source),input_sha256=core.sha(source),page_numbers=[3,1],mode='local',output_directory=str(root/'out'),config_version=core.CONFIG_VERSION)
            result=run_task(task,dict(apple_helper='/usr/bin/false'))
            self.assertEqual([p['status'] for p in result['page_results']],['NATIVE_PRESERVED','NATIVE_PRESERVED']);self.assertEqual(result['source_page_count'],3);self.assertEqual(result['exported_page_count'],3);self.assertEqual(result['untouched_page_numbers'],[2]);self.assertEqual(len(fitz.open(result['artifacts']['searchable_pdf'])),3)
            self.assertEqual(store.load_snapshot(root/'out')[0]['pdf_page_mapping'],result['pdf_page_mapping']);self.assertTrue(Path(result['artifacts']['page_mapping_json']).exists());self.assertFalse(list((root/'out/raw').glob('page-*/apple-input.json')))
