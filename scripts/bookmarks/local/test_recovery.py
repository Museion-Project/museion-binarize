import unittest
from recovery_layout import features, heading, spans, printed_ref, source_projection, visible_prefix


def raw(rows):
    return dict(source='/synthetic.pdf',source_sha256='a'*64,page_index=0,page_count=200,
                width=600,height=800,source_kind='native_text',state='inspected',
                bbox_convention='visible_top_left_points_xywh',observations=[
                    dict(id=f'o{i}',text=t,bbox=[x,y,w,12],tokens=[dict(text=s,bbox=b) for s,b in ts])
                    for i,(t,x,y,w,ts) in enumerate(rows)])


class RecoveryTests(unittest.TestCase):
    def test_visible_labels_remain_literal(self):
        for prefix in ['I.', 'I.1.', 'a.', 'α.', '3.2', '12', 'B)']:
            self.assertEqual(visible_prefix(prefix+' Actual title'),(prefix,'Actual title'))
        for title in ['Chapter Three', 'Ordinary title', 'A history of ideas']:
            self.assertEqual(visible_prefix(title),(None,title))

    def test_spaced_leader_tokens_do_not_hide_reference(self):
        r=raw([('Title . . . 9',70,150,160,[('Title',[70,150,90,12]),
               ('.',[170,150,3,12]),('.',[177,150,3,12]),('.',[184,150,3,12]),('9',[190,150,7,12])])])
        p,refs=source_projection(r,'/raw.json')
        self.assertEqual(list(refs.values()),['9'])
        self.assertEqual(p['lines'][0]['text'],'Title 9')

    def test_attached_native_reference_uses_real_glyphs(self):
        text='Title.....VIII'
        r=raw([(text,70,150,130,[(text,[70,150,130,12])])])
        r['glyphs']=[dict(id=f'g{i}',text=c,bbox=[70+i*10,150,8,12]) for i,c in enumerate(text)]
        r['observations'][0]['glyph_ids']=[g['id'] for g in r['glyphs']]
        before=repr(r)
        p,refs=source_projection(r,'/raw.json')
        self.assertEqual(list(refs.values()),['VIII'])
        self.assertEqual(p['lines'][0]['text'],'Title VIII')
        self.assertEqual(repr(r),before)
        r.pop('glyphs')
        _,refs=source_projection(r,'/raw.json')
        self.assertEqual(list(refs.values()),[None])

    def test_labeled_child_cannot_be_parent_continuation(self):
        for label in ['a.', 'α.', 'I.1.']:
            r=raw([('Container title',70,150,200,[]),(label+' Child title',70,170,200,[])])
            p,_=source_projection(r,'/raw.json')
            self.assertEqual(len(p['lines']),2,label)

    def test_caps_wrap_requires_same_style_and_no_new_label(self):
        r=raw([('A LONG UPPERCASE',70,150,200,[]),('WRAPPED TITLE',70,170,190,[])])
        p,_=source_projection(r,'/raw.json')
        self.assertEqual(len(p['lines']),1)
        r['observations'][1]['text']='Mixed case child'
        p,_=source_projection(r,'/raw.json')
        self.assertEqual(len(p['lines']),2)

    def test_explicit_heading_and_navigation_required(self):
        r=raw([('Contents',200,80,70,[]),('Introduction 1',70,130,400,[]),('Conclusion 30',70,160,400,[])])
        self.assertTrue(features(r)['seed'])
        r['observations'][0]['text']='The contents of this chapter are discussed below'
        self.assertFalse(features(r)['seed'])
        r['observations'][0]['text']='List of figures'
        self.assertFalse(features(r)['seed'])

    def test_distant_groups_and_continuation(self):
        fs=[dict(seed=False,continuation=False) for _ in range(12)]
        for i in [1,9]:fs[i]=dict(seed=True,continuation=True)
        fs[2]['continuation']=True
        self.assertEqual(spans(fs),[[1,2],[9]])

    def test_bibliography_entry_is_not_auxiliary_header(self):
        r=raw([('Contents',200,80,70,[]),('Bibliography',70,130,90,[]),
               ('20',480,130,20,[]),('Introduction',70,160,90,[]),('1',480,160,10,[])])
        f=features(r)
        self.assertTrue(f['seed'])
        self.assertEqual(f['auxiliary'],[])

    def test_headingless_numeric_prose_does_not_seed(self):
        r=raw([(f'This text cites ancient works ... {i+1}',70,100+i*20,420,[]) for i in range(10)])
        self.assertFalse(features(r)['seed'])

    def test_toc_region_stops_before_body_paragraph(self):
        r=raw([('Contents',200,80,70,[]),('Introduction 1',70,130,400,[]),('Conclusion 30',70,160,400,[])]+
              [('An ordinary paragraph discusses historical matters and continues across the full printed line.',70,200+i*16,430,[]) for i in range(5)])
        f=features(r)
        self.assertTrue(f['seed'])
        self.assertTrue(f['paragraph_boundary'])
        self.assertAlmostEqual(f['region_y'][1],200/800)

    def test_explicit_unpaginated_listing(self):
        r=raw([('Contents',200,80,70,[])]+[(t,70,130+i*25,270,[]) for i,t in enumerate(
            ['General historical studies','Early textual sources','Later textual sources','Appendices and supplements'])])
        self.assertTrue(features(r)['seed'])

    def test_later_seed_cannot_absorb_prior_body(self):
        self.assertEqual(spans([dict(seed=False,continuation=True),dict(seed=True,continuation=True)]),[[1]])

    def test_semantic_index_words_are_not_fuzzy_contents(self):
        for text in ['INDICES','Index','content','contentus','indices.']:
            self.assertFalse(heading(text),text)
        for text in ['Contents','CONTENT5','Inhaltsverzeichnis','Table des matières']:
            self.assertTrue(heading(text),text)

    def test_full_printed_reference(self):
        for s in ['1–8','ix–xii','3, 9','44']:
            self.assertEqual(printed_ref(s),s)
        for s in ['IIV','12x','chapter']:
            self.assertIsNone(printed_ref(s))

    def test_centered_variable_edge_and_range(self):
        r=raw([('Preface xvii',170,100,150,[('Preface',[170,100,75,12]),('xvii',[260,100,22,12])]),
               ('Some title 3–9',80,150,400,[('Some title',[80,150,180,12]),('3–9',[280,150,25,12])])])
        p,refs=source_projection(r,'/raw.json')
        self.assertEqual(list(refs.values()),['xvii','3–9'])
        self.assertEqual([l['printed_candidate'] for l in p['lines']],['xvii','3'])
        self.assertEqual(p['lines'][1]['text'],'Some title 3')

    def test_separate_numeric_row_provenance_survives(self):
        r=raw([('First title',70,100,100,[]),('8',480,100,12,[]),
               ('Second title',70,140,110,[]),('20',480,140,18,[])])
        p,refs=source_projection(r,'/raw.json')
        self.assertIn('o1',p['lines'][0]['raw_ids'])
        self.assertEqual(list(refs.values()),['8','20'])

    def test_outside_token_geometry_is_reviewable_not_invalid(self):
        r=raw([('Preface 4',-1,100,400,[('Preface',[-1,100,70,12]),('4',[350,100,10,12])])])
        p,_=source_projection(r,'/raw.json')
        self.assertEqual(p['lines'][0]['bbox']['x'],0)
        self.assertEqual(p['lines'][0]['state'],'ambiguous')


if __name__=='__main__':unittest.main()
