import copy,json,tempfile,unittest
from pathlib import Path
from unittest.mock import patch
import manager
import desktop_bridge as bridge

class ModelTests(unittest.TestCase):
    def test_apple_status_has_no_side_effects(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertFalse(manager.status(tmp,'apple')['available'])
            self.assertEqual(list(Path(tmp).iterdir()),[])
    def test_retired_providers_refused_before_runtime(self):
        with patch.object(manager,'apple_paths',side_effect=AssertionError('runtime')):
            for provider in ('minicpm-v4','qwen-vl-3b','deepseek-flash'):
                with self.assertRaises(ValueError):manager.status('/tmp',provider)
                with self.assertRaises(ValueError):bridge.project({},[],[],provider,Path('/tmp'),Path('/tmp'))
    def test_text_only_request_is_refused_before_runtime(self):
        with patch.object(manager,'status',side_effect=AssertionError('runtime')):
            with self.assertRaises(ValueError): manager.infer('/tmp','apple',{'mode':'text','images':[]},'/tmp')
    def test_unknown_provider_refused(self):
        with self.assertRaises(ValueError):manager.status('/tmp','remote-url')
        with self.assertRaises(ValueError):manager.status('/tmp','qwen-vl-3b')
    def test_cancellation_observed(self):
        with tempfile.TemporaryDirectory() as t:
            p=Path(t)/'cancel';p.touch()
            with self.assertRaises(InterruptedError):manager.check_cancel(p)
    def fixture(self):
        entries=[dict(id=str(i),title='DO NOT SEND TITLE',evidence_ids=[str(i)],section_label=None,printed_page='3',level=0,parent=None,target_pdf_page=6) for i in range(3)]
        table=dict(entries=copy.deepcopy(entries),source_entries=entries)
        page=dict(page_index=0,width=100,height=100,lines=[dict(id=str(i),bbox=dict(x=10,y=10+i*15,width=70,height=10)) for i in range(3)])
        return table,[page]
    def test_projection_only_changes_levels_and_parents_and_passes_images(self):
        table,pages=self.fixture();before=copy.deepcopy(table)
        reply=dict(status='completed',supported=True,relations=[dict(index=0,level=0,parent=-1),dict(index=1,level=1,parent=0),dict(index=2,level=1,parent=0)])
        with tempfile.TemporaryDirectory() as t,patch.object(bridge,'status',return_value={'available':True}),patch.object(bridge,'infer',return_value=reply) as infer:
            result,receipt=bridge.project(table,pages,['/tmp/source.png'],'apple',Path(t),Path(t))
            request=infer.call_args.args[2]
            self.assertNotIn('DO NOT SEND TITLE',request['prompt']);self.assertEqual(request['images'],[str(Path('/tmp/source.png').resolve())])
        self.assertEqual(table,before);self.assertEqual(receipt['status'],'applied')
        for k in table:
            self.assertEqual(result[k][1]['parent'],'0')
            for x,y in zip(result[k],before[k]):
                self.assertEqual({a:b for a,b in x.items() if a not in ('level','parent')},{a:b for a,b in y.items() if a not in ('level','parent')})
    def test_invalid_model_tree_does_not_partially_update(self):
        table,pages=self.fixture()
        with tempfile.TemporaryDirectory() as t,patch.object(bridge,'status',return_value={'available':True}),patch.object(bridge,'infer',return_value=dict(status='completed',supported=True,relations=[dict(index=0,level=0,parent=0)])):
            result,receipt=bridge.project(table,pages,['/tmp/source.png'],'apple',Path(t),Path(t))
        self.assertEqual(result,table);self.assertEqual(receipt['status'],'not_applied')
    def test_apple_unavailable_is_explicit_basic_result(self):
        table,pages=self.fixture()
        with tempfile.TemporaryDirectory() as t,patch.object(bridge,'status',return_value={'available':False,'message':'modelNotReady'}),patch.object(bridge,'infer',side_effect=AssertionError('inference')):
            result,receipt=bridge.project(table,pages,['/tmp/source.png'],'apple',Path(t),Path(t))
        self.assertEqual(result,table);self.assertIn('modelNotReady',receipt['message'])
    def test_navigation_membership_kept_after_source_parent_changes(self):
        table,pages=self.fixture();table['entries']=table['entries'][1:]
        reply=dict(status='completed',supported=True,relations=[dict(index=0,level=0,parent=-1),dict(index=1,level=1,parent=0),dict(index=2,level=2,parent=1)])
        with tempfile.TemporaryDirectory() as t,patch.object(bridge,'status',return_value={'available':True}),patch.object(bridge,'infer',return_value=reply):
            result,_=bridge.project(table,pages,['/tmp/source.png'],'apple',Path(t),Path(t))
        self.assertEqual([e['id'] for e in result['entries']],['1','2']);self.assertIsNone(result['entries'][0]['parent']);self.assertEqual(result['entries'][1]['level'],1)

if __name__=='__main__':unittest.main()
