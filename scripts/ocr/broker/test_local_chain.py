"""Opt-in actual Surya -> HTTP broker -> Rust compositor integration test."""
import json,os,subprocess,sys,threading,urllib.request,urllib.error
from pathlib import Path
import pytest
from server import Broker,Handler,HTTPServer,init_config
from test_broker import fake
ROOT=Path(__file__).resolve().parents[3]

@pytest.mark.skipif(os.environ.get('MPDF_RUN_LOCAL_SURYA_CHAIN')!='1',reason='explicit actual local model integration test')
def test_actual_surya_http_rust_chain(tmp_path):
    cfg=tmp_path/'private';init_config(cfg);(cfg/'gemini-api-key.txt').write_text('fake-gemini-key-no-network')
    calls=[]
    def upstream(payload,key):calls.append(payload);assert key=='fake-gemini-key-no-network';return fake(payload,key)
    broker=Broker(cfg,True,1,upstream);http=HTTPServer(('127.0.0.1',0),Handler);http.broker=broker
    worker=threading.Thread(target=http.serve_forever,daemon=True);worker.start();url=f'http://127.0.0.1:{http.server_port}'
    image=ROOT/'gold-data/mixed-scholarly-v1/images/burnet-platonis-opera-pdf1100.png';output=tmp_path/'ocr.json'
    try:
        request=urllib.request.Request(url+'/v1/transcription/pages',data=b'{}',headers={'Content-Type':'application/json'},method='POST')
        with pytest.raises(urllib.error.HTTPError) as e:urllib.request.urlopen(request)
        assert e.value.code==401 and not calls
        command=[sys.executable,str(ROOT/'scripts/ocr/broker/run_page.py'),str(image),str(output),'--broker',url,'--config-dir',str(cfg)]
        result=subprocess.run(command,cwd=ROOT,capture_output=True,text=True,timeout=240)
        assert result.returncode==0,result.stdout+result.stderr
        assert 'fake-gemini-key' not in result.stdout+result.stderr
        page=json.loads(output.read_text());envelope=json.loads(output.with_suffix('.geometry.json').read_text())
        assert json.loads(page['provider_raw_artifact'])==envelope['evidence']
        geometry=envelope['geometry'];actual=page['blocks'][0]['lines']
        assert len(actual)==len(geometry['lines'])==40 and len(calls)==1
        for line,g in zip(actual,geometry['lines']):assert line['bbox']==g['bbox'] and line['reading_order']==g['reading_order']
        assert page['provider_provenance']['engine']=='surya-line-geometry+google-gemini'
        assert page['provider_provenance']['parameters']['contract']=='mpdf-geometry-transcription/1'
        again=tmp_path/'repeat.json'
        cmd=[str(ROOT/'target/debug/examples/surya_broker_page'),str(image),str(output.with_suffix('.geometry.json')),str(again),url,str(cfg/'client.token')]
        result=subprocess.run(cmd,capture_output=True,text=True,timeout=30);assert result.returncode==0,result.stderr
        assert len(calls)==1 and json.loads(again.read_text())==page
        # A changed logical line cannot be silently sent under the same evidence.
        envelope['geometry']['lines'][0]['bbox']['x']+=1;bad=tmp_path/'bad.json';bad.write_text(json.dumps(envelope));cmd[2]=str(bad);cmd[3]=str(tmp_path/'bad-output.json')
        result=subprocess.run(cmd,capture_output=True,text=True,timeout=30);assert result.returncode!=0 and len(calls)==1
    finally:http.shutdown();http.server_close();worker.join();broker.db.close()
