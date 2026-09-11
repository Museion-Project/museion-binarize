"""Hierarchy projection after the unchanged OCR/compiler/pagination stages."""
import copy, json, sys
from pathlib import Path
from manager import status, infer, atomic, default_root, check_cancel, IDS
# Packaged with this adapter; source checkout imports the original pilot contract.
contract_dir=Path(__file__).resolve().parent.parent/'hierarchy_foundation_models'
sys.path.insert(0,str(contract_dir))
from hierarchy import group_request, apply_relations


def project(table, pages, images, model, model_root, work, cancel=None):
    result=copy.deepcopy(table)
    receipt=dict(provider=model,status='not_applied',message='',scope='hierarchy_only_source_images')
    if model not in IDS: raise ValueError('Unknown hierarchy provider')
    state=status(model_root,model)
    if not state['available']:
        receipt['message']='Apple 暂不可用，本次保留基础目录层级：'+state.get('message','')
        return result,receipt
    source=table.get('source_entries') or table['entries']
    if not source:
        receipt['message']='没有可供模型判断的目录条目';return result,receipt
    if len(source)>60 or len(pages)>3:
        receipt['message']='当前模型试用支持最多 3 页、60 个条目；本次保留基础层级。';return result,receipt
    group={'entries':[dict(entry_id=e['id'],evidence_ids=e['evidence_ids'],
             numbering=e.get('section_label'),printed_page_ref=e.get('printed_page'),level=e['level'],parent_entry_id=e['parent']) for e in source]}
    try:
        request=group_request(group,pages,images,'image')
        atomic(work/'hierarchy-input.json',request)
        reply=infer(model_root,model,request,work/'model',cancel)
        atomic(work/'hierarchy-response.json',reply)
        updated,applied=apply_relations(group,reply)
        receipt.update(status=applied,seconds=reply.get('seconds'))
        if applied != 'applied': receipt['message']='模型未给出可用层级，本次保留基础层级。';return result,receipt
        relations={e['entry_id']:e for e in updated['entries']}
        # Keep exactly the existing source and navigation memberships. Navigation
        # ancestors omitted by the compiler are walked through, never reinserted.
        for key in ('source_entries','entries'):
            entries=result.get(key,[]); included={e['id'] for e in entries}; levels={}
            for e in entries:
                parent=relations[e['id']]['parent_entry_id']
                while parent is not None and parent not in included: parent=relations[parent]['parent_entry_id']
                e['parent']=parent;e['level']=0 if parent is None else levels[parent]+1
                levels[e['id']]=e['level']
        receipt['message']='已生成模型层级建议，请逐项核对后保存。'
    except InterruptedError: raise
    except Exception as e:
        result=copy.deepcopy(table);receipt.update(status='not_applied',error=str(e),message='模型未返回有效的目录层级，本次保留基础层级，请人工核对。')
    check_cancel(cancel)
    return result,receipt


def main():
    verb,path=sys.argv[1:]; req=json.loads(Path(path).read_text()); work=Path(req['work_dir'])
    root=Path(req.get('model_root') or default_root()); cancel=work/'cancel'
    if verb=='models_status': result=[status(root,m) for m in IDS]
    elif verb=='hierarchy':
        finished=json.loads((work/'finish-result.json').read_text())
        pages=json.loads((work/'evidence.json').read_text())
        rendered=json.loads((work/'prepare-request.json').read_text())['rendered_pages']
        images=[rendered[str(p['page_index']+1)]['image_path'] for p in pages]
        atomic(work/'progress.json',{'stage':'interpreting_hierarchy'})
        table,receipt=project(finished['table'],pages,images,req['provider'],root,work,cancel)
        result=dict(table=table,intake=dict(finished['intake'],hierarchy_provider=req['provider'],hierarchy=receipt))
        atomic(work/'hierarchy-receipt.json',receipt)
    else: raise ValueError('Unknown operation')
    atomic(work/f'{verb}-result.json',result)
if __name__=='__main__':main()
