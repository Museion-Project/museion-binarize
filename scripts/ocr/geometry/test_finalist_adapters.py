import copy
import pytest
from PIL import Image,ImageDraw
from finalist_adapters import normalize,validate,adaptive_bands

def payload(boxes,regions=None):
    return {'detection':{'dt_polys':[[[a,b],[c,b],[c,d],[a,d]] for a,b,c,d in boxes],
                         'dt_scores':[.9]*len(boxes)},'layout':{'boxes':regions or []}}

def test_scale_invariant_narrow_gutter_and_local_band():
    boxes=[[0,0,202,10],[0,150,202,160]]+[[x,y,x+100,y+10] for x in [0,102] for y in [40,60,80,100,120]]
    counts=[]
    for s in [1,3.7]:
        fs=[{'fragment_id':str(i),'bbox':[v*s for v in b]} for i,b in enumerate(boxes)]
        bands=adaptive_bands(fs);counts.append(len(bands))
        assert len(bands)==1 and abs(bands[0]['cut_x']/s-101)<2
    assert counts==[1,1]

def test_toc_fragments_group_without_losing_raw_boxes():
    image=Image.new('RGB',(500,200),'white');draw=ImageDraw.Draw(image)
    boxes=[[20,40,180,60],[420,40,440,60]]
    for x in range(200,415,10):draw.rectangle((x,54,x+2,56),fill='black')
    raw=payload(boxes);d=normalize('paddle',raw,image,'synthetic',{})
    assert len(d['logical_lines'])==1 and len(d['fragments'])==2
    assert d['provider_raw']==raw
    assert any(h['kind']=='dotted_row_candidate' for h in d['logical_lines'][0]['hints'])
    assert normalize('paddle',raw,image,'synthetic',{})['geometry_sha256']==d['geometry_sha256']

def test_recurring_margin_split_without_text():
    image=Image.new('RGB',(500,450),'white');draw=ImageDraw.Draw(image);boxes=[]
    for i in range(12):
        y=20+i*32;merged=i in [1,3,6,9]
        draw.rectangle((30,y,350,y+18),fill='black')
        if merged:draw.rectangle((399,y+3,407,y+15),fill='black')
        boxes.append([30,y,410 if merged else 350,y+20])
    d=normalize('paddle',payload(boxes),image,'synthetic',{})
    assert len(d['margin_lanes'])==1 and len(d['derivations'])==4
    assert len(d['fragments'])==16 and len(d['logical_lines'])==16
    validate(d)
    broken=copy.deepcopy(d);broken['fragments'].pop()
    with pytest.raises((AssertionError,KeyError)):validate(broken)

def test_paddle_preserves_nested_region_graph():
    boxes=[[x,y,x+100,y+10] for x in [10,120] for y in [20,40,60,80,100]]
    regs=[{'coordinate':[8,18,112,115],'label':'native_uninterpreted','score':.9},
          {'coordinate':[118,18,225,115],'label':'native_uninterpreted','score':.9},
          {'coordinate':[9,19,110,55],'label':'reference_content','score':.7}]
    d=normalize('paddle',payload(boxes,regs),Image.new('RGB',(250,150),'white'),'synthetic',{})
    assert d['column_method']=='provider_layout_containment'
    assert d['layout_regions'][0]['contains_region_ids']==['r0002']
    assert d['provider_raw']['layout']['boxes']==regs
    assert len(d['column_bands'])==1

def test_surya_full_payload_and_no_layout_remain_traceable():
    raw={'detection':{'bboxes':[{'polygon':[[10,10],[100,10],[100,20],[10,20]],'bbox':[10,10,100,20],'confidence':.8}],
                      'image_bbox':[0,0,200,100],'heatmap':None,'affinity_map':None},'layout':None}
    d=normalize('surya',raw,Image.new('RGB',(200,100),'white'),'synthetic',{})
    assert d['provider_raw']==raw and d['source_fragments'][0]['confidence']==.8
    broken=copy.deepcopy(d);broken['source_fragments'][0]['polygon'][0][0]=99
    with pytest.raises(AssertionError):validate(broken)


def test_surya_multiline_split_preserves_polygon_and_attaches_small_row_fragment():
    image=Image.new('RGB',(600,400),'white');draw=ImageDraw.Draw(image)
    boxes=[[20,y,520,y+20] for y in [10,40,70,100,130,160]]
    boxes += [[30,210,520,290],[15,238,60,251]]
    for b in boxes[:-2]:draw.rectangle(b,fill='black')
    for y in [215,239,263]:draw.rectangle([30,y,510,y+13],fill='black')
    draw.rectangle([15,239,60,250],fill='black')
    raw={'detection':{'bboxes':[{'bbox':b,'polygon':[[b[0],b[1]],[b[2],b[1]],[b[2],b[3]],[b[0],b[3]]],'confidence':.9} for b in boxes]},'layout':None}
    d=normalize('surya',raw,image,'synthetic',{})
    assert d['provider_raw']==raw
    events=[e for e in d['derivations'] if e['operation']=='ink_valley_row_split']
    assert len(events)==1 and len(events[0]['output_fragment_ids'])==3
    rows=[l for l in d['logical_lines'] if l['bbox'][1]>=210]
    assert len(rows)==3
    assert 'd0007' in rows[1]['fragment_ids']
    validate(d)
    assert normalize('surya',raw,image,'synthetic',{})['geometry_sha256']==d['geometry_sha256']
    # The shared Paddle path does not adopt this repair.
    pd=normalize('paddle',payload(boxes),image,'synthetic',{})
    assert not any(e['operation']=='ink_valley_row_split' for e in pd['derivations'])


def test_surya_unsplittable_tall_box_does_not_swallow_small_box():
    image=Image.new('RGB',(600,400),'white')
    boxes=[[20,y,520,y+20] for y in [10,40,70,100,130,160]]+[[30,210,520,290],[15,238,60,251]]
    raw={'detection':{'bboxes':[{'bbox':b,'polygon':[[b[0],b[1]],[b[2],b[1]],[b[2],b[3]],[b[0],b[3]]]} for b in boxes]},'layout':None}
    d=normalize('surya',raw,image,'synthetic',{})
    assert not any(e['operation']=='ink_valley_row_split' for e in d['derivations'])
    assert any(l['fragment_ids']==['d0007'] for l in d['logical_lines'])
