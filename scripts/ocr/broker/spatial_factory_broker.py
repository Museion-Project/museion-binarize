"""Foreground loopback broker for operational checked /2 small batches.

Only this process reads the model credential. State is exclusive per invocation;
failed/uncertain pages never retry. Historical /1 has a different endpoint and
cannot service this client. No production billing or Credits claim is made.
"""
import argparse
import base64
import hmac
import json
import os
from pathlib import Path
import re
import time
import urllib.request
from http.server import BaseHTTPRequestHandler, HTTPServer

import spatial_verification as v
import spatial_batch_strategy as batches
import spatial_batch_audit as audit
from server import MODEL, CONFIG, private_read, NoRedirect

PROTOCOL = 'mpdf-spatial-factory-broker/1'


def google_raw(payload, key):
    request = urllib.request.Request(f'https://generativelanguage.googleapis.com/v1beta/models/{MODEL}:generateContent',
        data=batches.encoded(payload),headers={'Content-Type':'application/json','x-goog-api-key':key},method='POST')
    with urllib.request.build_opener(urllib.request.ProxyHandler({}),NoRedirect()).open(request,timeout=180) as response:
        raw = response.read(8*1024*1024+1)
    v.require(len(raw)<=8*1024*1024,'Upstream response too large')
    return raw


def prepare_request(r):
    v.require(set(r)=={'protocol','idempotency_key','model','geometry_json','parent_json','spatial_json',
                      'image_base64','expected','receipts','plan_sha256','max_batches','input_stop'},'Unexpected /2 request fields')
    v.require(r['protocol']==PROTOCOL and r['model']==MODEL,'Spatial protocol/model mismatch')
    v.require(isinstance(r['idempotency_key'],str) and re.fullmatch('[A-Za-z0-9_-]{1,256}',r['idempotency_key']),'Request identity')
    expected = r['expected']
    names = set(v.ExpectedArtifacts.__dataclass_fields__)
    v.require(set(expected)==names and type(r['max_batches']) is int and r['max_batches']>0
              and type(r['input_stop']) is int and r['input_stop']>0,'Invalid selected budget/bindings')
    png = base64.b64decode(r['image_base64'],validate=True)
    v.require(len(png)<=32*1024*1024,'Page image too large')
    receipts = {name:raw.encode() for name,raw in r['receipts'].items()}
    p = v.prepare(r['geometry_json'],r['parent_json'].encode(),r['spatial_json'].encode(),png,
                  v.ExpectedArtifacts(**expected),receipts)
    plan = batches.encoded(batches.plan(p,MODEL))
    batches.verify_plan(p,plan,r['plan_sha256'])
    v.require(len(v.parse(plan)['batches'])<=r['max_batches'],'Page batch ceiling exceeded')
    return p,plan


class SpatialBroker:
    def __init__(self,state_dir,config_dir=CONFIG,allow_paid=False,max_pages=0,max_batches=0,input_stop=0,upstream=google_raw):
        self.state_dir = Path(state_dir)
        self.state_dir.mkdir(parents=True,exist_ok=False,mode=0o700)
        self.config_dir = Path(config_dir)
        self.token = private_read(self.config_dir/'client.token')
        v.require(len(self.token)>=32,'Invalid client token')
        self.allow_paid,self.max_pages,self.max_batches,self.input_stop = allow_paid,max_pages,max_batches,input_stop
        self.upstream = upstream
        self.pages = 0
        self.used_batches = 0
        self.input_used = 0
        self.unknown_usage = False
        self.code_sha256 = v.sha(Path(__file__).read_bytes())
        self.persist()

    def status(self):
        return {'protocol':PROTOCOL,'model':MODEL,'allow_paid':self.allow_paid,'max_pages':self.max_pages,
                'max_batches':self.max_batches,'input_stop':self.input_stop,'used_pages':self.pages,
                'used_batches':self.used_batches,'recorded_input_tokens':self.input_used,
                'usage_incomplete':self.unknown_usage,'code_sha256':self.code_sha256,'monetary_cost':None}

    def persist(self):
        temp = self.state_dir/'status.tmp'
        temp.write_bytes(batches.encoded(self.status()))
        temp.replace(self.state_dir/'status.json')

    def handle(self,r):
        p,plan = prepare_request(r)
        count = len(v.parse(plan)['batches'])
        directory = self.state_dir/r['idempotency_key']
        if directory.exists():
            return 409,{'error':'Page already attempted; no implicit retry or cached model fill'}
        if not self.allow_paid:
            return 403,{'error':'Paid calls disabled'}
        if self.pages>=self.max_pages or self.used_batches+count>self.max_batches or self.unknown_usage or self.input_used>self.input_stop:
            return 402,{'error':'Explicit broker budget exceeded or usage unknown'}
        key = private_read(self.config_dir/'gemini-api-key.txt')
        v.require(key and '\n' not in key,'Model credential unavailable')
        directory.mkdir()
        batches.save(directory/'request.json',r)
        self.pages += 1
        self.persist()
        started = time.monotonic()
        def transport(payload):
            v.require(self.used_batches<self.max_batches and self.input_used<=self.input_stop and not self.unknown_usage,'Global batch/token stop')
            self.used_batches += 1
            self.persist()
            try:
                raw = self.upstream(payload,key)
            except Exception:
                self.unknown_usage = True
                self.persist()
                raise
            try:
                usage = v.parse(raw).get('usageMetadata',{})
                n = usage.get('promptTokenCount')
                if type(n) is int and n>=0:
                    self.input_used += n
                else:
                    self.unknown_usage = True
            except Exception:
                self.unknown_usage = True
            self.persist()
            print(v.canonical({'event':'batch_returned','used_batches':self.used_batches,'input_tokens':self.input_used}),flush=True)
            return raw
        try:
            result = batches.execute(p,plan,r['plan_sha256'],directory/'execution',transport,r['input_stop'])
            aggregation_sha = v.sha((directory/'execution/aggregation.json').read_bytes())
            response,receipt = audit.verify_completed_run(p,directory/'execution',r['plan_sha256'],aggregation_sha)
            files = {str(path.relative_to(directory/'execution')):path.read_text()
                     for path in sorted((directory/'execution').rglob('*')) if path.is_file()}
            result = {'schema':'mpdf-spatial-broker-result/1','protocol':PROTOCOL,'model':MODEL,
                      'idempotency_key':r['idempotency_key'],'request_sha256':v.sha(batches.encoded(r)),
                      'plan_sha256':r['plan_sha256'],'aggregation_sha256':aggregation_sha,
                      'response_sha256':v.sha(response.encode()),'complete_page':True,'files':files,
                      'elapsed_seconds':time.monotonic()-started,'usage':receipt['usage'],
                      'automatic_retries':0,'historical_fill_supports':0,'monetary_cost':None}
            batches.save(directory/'result.json',result)
            return 200,result
        except Exception as error:
            batches.save(directory/'failure.json',{'state':'failed_or_uncertain','error_type':type(error).__name__,
                          'automatic_retry':False,'elapsed_seconds':time.monotonic()-started})
            return 502,{'error':'Spatial batch failed or response invalid; inspect retained evidence; no retry'}


class Handler(BaseHTTPRequestHandler):
    def log_message(self,*args):
        pass

    def authorized(self):
        return not self.headers.get('Origin') and hmac.compare_digest(self.headers.get('Authorization',''),'Bearer '+self.server.broker.token)

    def do_GET(self):
        if not self.authorized():
            return self.reply(401,{'error':'Unauthorized'})
        if self.path=='/v2/spatial/status':
            return self.reply(200,self.server.broker.status())
        self.reply(404,{'error':'Unknown endpoint'})

    def do_POST(self):
        if self.path!='/v2/spatial/pages':
            return self.reply(404,{'error':'Only checked /2 small-batch endpoint is supported'})
        if not self.authorized():
            return self.reply(401,{'error':'Unauthorized'})
        try:
            length = int(self.headers.get('Content-Length','0'))
            v.require(0<length<=128*1024*1024,'Request size')
            self.connection.settimeout(60)
            r = v.parse(self.rfile.read(length))
            status,result = self.server.broker.handle(r)
        except Exception:
            status,result = 400,{'error':'Invalid selected spatial request'}
        self.reply(status,result)

    def reply(self,status,result):
        raw=batches.encoded(result)
        self.send_response(status)
        self.send_header('Content-Type','application/json')
        self.send_header('Content-Length',str(len(raw)))
        self.send_header('Cache-Control','no-store')
        self.end_headers()
        self.wfile.write(raw)


def main():
    p=argparse.ArgumentParser()
    p.add_argument('--state-dir',type=Path,required=True)
    p.add_argument('--config-dir',type=Path,default=CONFIG)
    p.add_argument('--port',type=int,default=18767)
    p.add_argument('--allow-paid',action='store_true')
    p.add_argument('--max-pages',type=int,default=0)
    p.add_argument('--max-batches',type=int,default=0)
    p.add_argument('--input-stop',type=int,default=0)
    a=p.parse_args()
    v.require(not a.allow_paid or min(a.max_pages,a.max_batches,a.input_stop)>0,'Explicit bounded budget required')
    os.umask(0o077)
    # Bind first: an occupied port must not create a misleading running ledger.
    server=HTTPServer(('127.0.0.1',a.port),Handler)
    try:
        server.broker=SpatialBroker(a.state_dir,a.config_dir,a.allow_paid,a.max_pages,a.max_batches,a.input_stop)
        print(v.canonical({'event':'spatial_broker_listening','port':a.port,'paid':a.allow_paid}),flush=True)
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


if __name__=='__main__':
    try:
        main()
    except Exception:
        sys.exit('Spatial broker failed; credential and upstream error details suppressed')
