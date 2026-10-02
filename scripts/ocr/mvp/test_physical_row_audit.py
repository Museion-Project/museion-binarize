"""Physical consumer rows, especially aggregate nodes and repeated tokens."""
import unittest
from .pdf_consumer_audit import physical_row_relations


class PhysicalRowTests(unittest.TestCase):
    def members(self):
        return [dict(id='a',text='qualify',bbox=[0,0,60,10]),
                dict(id='b',text='the',bbox=[60,0,90,10]),
                dict(id='c',text='the',bbox=[100,0,130,10]),
                dict(id='d',text='the',bbox=[0,50,30,60])]

    def ledger(self):
        return dict(rows=[dict(row_id='body',bbox=[0,0,90,10],role='body',column_id='left',member_ids=['a','b']),
                          dict(row_id='margin',bbox=[100,0,130,10],role='margin',column_id='right',member_ids=['c']),
                          dict(row_id='foot',bbox=[0,50,30,60],role='footnote',column_id='left',member_ids=['d'])])

    def node(self,n,text,box):
        return dict(node_id=n,raw_text=text,bbox=box,tokens=[dict(text=t) for t in text.split()])

    def nodes(self):
        return [self.node(0,'qualify the',[0,0,90,10]),self.node(1,'the',[100,0,130,10]),self.node(2,'the',[0,50,30,60])]

    def test_aggregate_proves_row_while_word_precision_stays_unknown(self):
        result=physical_row_relations(self.members(),self.nodes(),self.ledger())
        self.assertEqual(result['state'],'PASS')
        self.assertEqual(result['nodes'][0]['binding']['member_ids'],['a','b'])
        self.assertFalse(result['individual_positions_proven'])

    def test_same_y_other_column_cannot_borrow_body_members(self):
        nodes=self.nodes();nodes[0]['bbox']=[100,0,190,10]
        result=physical_row_relations(self.members(),nodes,self.ledger())
        self.assertEqual(result['state'],'INSUFFICIENT')
        self.assertEqual(result['unresolved_member_ids'],['a','b'])

    def test_node_spanning_body_and_footnote_does_not_pass(self):
        nodes=self.nodes();nodes[0]['bbox']=[0,0,90,60]
        self.assertEqual(physical_row_relations(self.members(),nodes,self.ledger())['state'],'INSUFFICIENT')

    def test_duplicated_consumer_occurrence_cannot_reuse_source(self):
        nodes=self.nodes()+[self.node(3,'the',[0,50,30,60])]
        result=physical_row_relations(self.members(),nodes,self.ledger())
        self.assertEqual(result['state'],'INSUFFICIENT')
        self.assertIn('d',result['unresolved_member_ids'])

    def test_complete_nodes_in_wrong_reading_order_fail(self):
        result=physical_row_relations(self.members(),list(reversed(self.nodes())),self.ledger())
        self.assertEqual(result['state'],'FAIL')
        self.assertTrue(result['complete_membership'])

    def test_missing_bounds_are_insufficient_and_invalid_members_rejected(self):
        ledger=self.ledger();ledger['rows'][0].pop('bbox')
        self.assertEqual(physical_row_relations(self.members(),self.nodes(),ledger)['state'],'INSUFFICIENT')
        ledger=self.ledger();ledger['rows'][1]['member_ids'].append('b')
        with self.assertRaisesRegex(ValueError,'SOURCE_LEDGER_MEMBERS'):
            physical_row_relations(self.members(),self.nodes(),ledger)

    def test_ambiguous_repeated_member_span_keeps_unknown(self):
        source=[dict(id='a',text='the',bbox=[0,0,30,10]),dict(id='b',text='the',bbox=[0,0,30,10])]
        ledger=dict(rows=[dict(row_id='r',bbox=[0,0,30,10],role='body',column_id='left',member_ids=['a','b'])])
        result=physical_row_relations(source,[self.node(0,'the',[0,0,30,10]),self.node(1,'the',[0,0,30,10])],ledger)
        self.assertEqual(result['state'],'INSUFFICIENT')


if __name__=='__main__':unittest.main()
