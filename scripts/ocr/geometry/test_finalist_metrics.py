from finalist_adapters import clip_polygon_x
from score_finalists import union_area,support,page_metrics

def test_fragment_union_does_not_credit_empty_envelope():
    assert union_area([[0,0,2,2],[1,0,3,2]])==6
    assert support([0,0,10,10],[[0,0,1,10],[9,0,10,10]])=={'coverage':.2,'iou':.2}

def test_split_polygon_stays_in_child_extent():
    source=[[0,1],[10,0],[10,9],[0,10]]
    left=clip_polygon_x(source,0,4);right=clip_polygon_x(source,4,10)
    assert all(0<=p[0]<=4 for p in left)
    assert all(4<=p[0]<=10 for p in right)
    assert [4,0.6] in left and [4,0.6] in right

def test_column_accuracy_and_pair_coverage_are_separate():
    ref=[{'line_id':str(i),'bbox':[x,y,x+10,y+5],'reading_order':i,'column':c,'category':None}
         for i,(x,y,c) in enumerate([(0,0,'L'),(0,10,'L'),(20,0,'R'),(20,10,'R')])]
    order=[0,2,1,3]
    fs=[{'fragment_id':str(i),'bbox':r['bbox']} for i,r in enumerate(ref)]
    doc={'page_id':'synthetic','logical_lines':[{'line_id':str(i),'bbox':ref[i]['bbox'],'fragment_ids':[str(i)],'column_id':ref[i]['column'],'hints':[]} for i in order],
         'fragments':fs,'source_fragments':fs,'column_bands':[],'column_method':'test','margin_lanes':[],'derivations':[]}
    m=page_metrics(ref,doc)
    assert m['column']['pair_accuracy']==1
    assert m['column']['within_inverted']==0 and m['column']['cross_inverted']==1
    assert m['inverted_pairs']==1
