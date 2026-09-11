import copy
import unittest

from hierarchy import apply_relations, group_request


class HierarchyContractTests(unittest.TestCase):
    def setUp(self):
        self.group = {'entries': [dict(entry_id=f'e{i}', title=f'SECRET TITLE {i}',
                                      printed_page_ref='ix', order=i+1, level=0,
                                      parent_entry_id=None, evidence_ids=[f'r{i}'], numbering=None)
                                  for i in range(3)], 'physical_pages': [4]}

    def reply(self, relations):
        return dict(status='completed', supported=True,
                    relations=[dict(index=i, level=level, parent=parent)
                               for i,(level,parent) in enumerate(relations)])

    def test_only_hierarchy_can_change(self):
        before=copy.deepcopy(self.group)
        after,status=apply_relations(self.group,self.reply([(0,-1),(1,0),(2,1)]))
        self.assertEqual(status,'applied');self.assertEqual(self.group,before)
        for old,new in zip(before['entries'],after['entries']):
            self.assertEqual({k:v for k,v in old.items() if k not in ('level','parent_entry_id')},
                             {k:v for k,v in new.items() if k not in ('level','parent_entry_id')})

    def test_reject_cycle_or_forward_parent(self):
        with self.assertRaises(ValueError):apply_relations(self.group,self.reply([(0,-1),(1,2),(1,0)]))

    def test_reject_disagreeing_depth(self):
        with self.assertRaises(ValueError):apply_relations(self.group,self.reply([(0,-1),(2,0),(1,0)]))

    def test_reject_closed_subtree(self):
        with self.assertRaises(ValueError):apply_relations(self.group,self.reply([(0,-1),(0,-1),(1,0)]))

    def test_reject_missing_or_extra_entry(self):
        with self.assertRaises(ValueError):apply_relations(self.group,self.reply([(0,-1)]))

    def test_unavailable_and_abstain_preserve_baseline(self):
        for status in ['unavailable','error','abstained']:
            after,receipt=apply_relations(self.group,dict(status=status))
            self.assertEqual(after,self.group);self.assertTrue(receipt.startswith('not_applied:'))

    def test_prompt_has_no_title_or_baseline_hierarchy(self):
        pages=[dict(page_index=3,width=100,height=200,
                    lines=[dict(id=f'r{i}',text='DO NOT TRANSMIT ME',bbox=dict(x=10,y=20+i*10,width=60,height=8)) for i in range(3)])]
        request=group_request(self.group,pages,['/tmp/page.jpg'],'image')
        self.assertNotIn('SECRET TITLE',request['prompt']);self.assertNotIn('DO NOT TRANSMIT ME',request['prompt'])
        self.assertNotIn('"level":',request['prompt']);self.assertNotIn('"parent_entry_id":',request['prompt'])
        with self.assertRaises(ValueError):group_request(self.group,pages,[],'features')

    def test_missing_geometry_never_guessed(self):
        with self.assertRaises(ValueError):group_request(self.group,[],[],'image')


if __name__=='__main__':unittest.main()
