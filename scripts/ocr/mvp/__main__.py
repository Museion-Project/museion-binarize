import argparse
import json
from .local import readiness, run_task, cancel_task
from .store import read, review_save, load_snapshot

def main():
    p=argparse.ArgumentParser(description='Opt-in fresh local OCR; JSON stdout, progress stderr')
    p.add_argument('command',choices=['readiness','run','review-save','reload','cancel']);p.add_argument('--task');p.add_argument('--config');p.add_argument('--mode',choices=['local','apple-residual'],default='local');p.add_argument('--output');p.add_argument('--patch')
    a=p.parse_args();config=read(a.config) if a.config else None
    if a.command=='readiness':result=readiness(a.mode,config)
    elif a.command=='run':result=run_task(read(a.task),config)
    elif a.command=='review-save':result=review_save(a.output,read(a.patch))
    elif a.command=='cancel':result=cancel_task(a.output)
    else:
        snapshot,folder=load_snapshot(a.output);result=dict(schema_version=1,revision=snapshot['revision'],snapshot=snapshot,folder=str(folder))
    print(json.dumps(result,ensure_ascii=False))
if __name__=='__main__':main()
