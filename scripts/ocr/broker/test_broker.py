import base64,copy,hashlib,io,json,os,threading
from pathlib import Path
import pytest
from PIL import Image
from server import Broker,Handler,HTTPServer,MODEL,PROTOCOL,check_request,init_config,private_read,transcribe,validate_text

def request():
    im=Image.new('RGB',(100,100),'white');f=io.BytesIO();im.save(f,format='PNG');png=f.getvalue()
    g={'page_index':0,'width':100,'height':100,'image_sha256':'a'*64,'provider_id':'surya-line-geometry','provider_version':'surya-0.17.0/text_detection-2025_05_07/r3.0-apparatus-repair','validation_status':'historical_material_not_validated','lines':[{'line_id':'a','reading_order':0,'bbox':{'x':1,'y':1,'width':20,'height':10}},{'line_id':'b','reading_order':1,'bbox':{'x':1,'y':20,'width':20,'height':10}}]}
    gj=json.dumps(g,separators=(',',':'));return {'protocol':PROTOCOL,'model':MODEL,'geometry_json':gj,'geometry_sha256':hashlib.sha256(gj.encode()).hexdigest(),'page_png_base64':base64.b64encode(png).decode(),'upload_sha256':hashlib.sha256(png).hexdigest(),'language_profile':'mixed','idempotency_key':'test-1'}

def fake(payload,key):
    contract=json.loads(payload['contents'][0]['parts'][0]['text'].split('; contract: ')[1]);result={'page_index':contract['page_index'],'geometry_sha256':contract['geometry_sha256'],'lines':[{'line_id':l['line_id'],'text':'λόγος','language':'grc','confidence':.9} for l in contract['lines']]}
    return {'modelVersion':MODEL,'candidates':[{'finishReason':'STOP','content':{'parts':[{'text':json.dumps(result)}]}}],'usageMetadata':{'promptTokenCount':30,'candidatesTokenCount':15}}

@pytest.fixture
def cfg(tmp_path):
    p=tmp_path/'broker';init_config(p);(p/'gemini-api-key.txt').write_text('never-echo-this-test-key');return p

def test_private_files_and_non_overwrite(cfg):
    init_config(cfg);assert private_read(cfg/'gemini-api-key.txt')=='never-echo-this-test-key'
    os.chmod(cfg/'gemini-api-key.txt',0o644)
    with pytest.raises(ValueError):private_read(cfg/'gemini-api-key.txt')

def test_disabled_missing_key_budget_and_idempotency(cfg):
    calls=[]
    def upstream(payload,key):calls.append(1);return fake(payload,key)
    b=Broker(cfg,False,1,upstream);r=request();assert b.handle(r)[0]==403 and not calls
    b.allow_paid=True;(cfg/'gemini-api-key.txt').write_text('');assert b.handle(r)[0]==503
    (cfg/'gemini-api-key.txt').write_text('never-echo-this-test-key');status,result=b.handle(r);assert status==200 and len(calls)==1
    assert b.handle(r)==(200,result) and len(calls)==1
    changed={**r,'language_profile':'other'};assert b.handle(changed)[0]==409
    other={**r,'idempotency_key':'new'};assert b.handle(other)[0]==402
    b.db.close();restarted=Broker(cfg,True,1,upstream);assert restarted.handle(r)==(200,result) and len(calls)==1

def test_unknown_outcome_is_never_retried_or_exposed(cfg):
    calls=[]
    def fail(payload,key):calls.append(1);raise RuntimeError(key+' private request contents')
    b=Broker(cfg,True,2,fail);r=request();status,result=b.handle(r)
    assert status==502 and 'never-echo' not in json.dumps(result)
    assert b.handle(r)[0]==409 and len(calls)==1

@pytest.mark.parametrize('kind',['digest','image','order','coordinate','duplicate'])
def test_invalid_input_rejected_before_upstream(cfg,kind):
    b=Broker(cfg,True,1,lambda *_:pytest.fail('unexpected call'));r=request()
    if kind=='digest':r['geometry_sha256']='bad'
    elif kind=='image':r['upload_sha256']='bad'
    else:
        g=json.loads(r['geometry_json'])
        if kind=='order':g['lines'][0]['reading_order']=1
        if kind=='coordinate':g['lines'][0]['bbox']['x']=-1
        if kind=='duplicate':g['lines'][1]['line_id']='a'
        r['geometry_json']=json.dumps(g);r['geometry_sha256']=hashlib.sha256(r['geometry_json'].encode()).hexdigest()
    with pytest.raises(ValueError):b.handle(r)

@pytest.mark.parametrize('kind',['order','missing','coordinates','digest','truncated','version'])
def test_invalid_gemini_output_rejected(kind):
    r=request();g=check_request(r)
    def upstream(payload,key):
        data=fake(payload,key);candidate=data['candidates'][0];result=json.loads(candidate['content']['parts'][0]['text'])
        if kind=='order':result['lines'].reverse()
        if kind=='missing':result['lines'].pop()
        if kind=='coordinates':result['lines'][0]['bbox']=[0,0,1,1]
        if kind=='digest':result['geometry_sha256']='wrong'
        if kind=='truncated':candidate['finishReason']='MAX_TOKENS'
        if kind=='version':data['modelVersion']='other-model'
        candidate['content']['parts'][0]['text']=json.dumps(result);return data
    with pytest.raises(ValueError):transcribe(r,g,'test',upstream)
