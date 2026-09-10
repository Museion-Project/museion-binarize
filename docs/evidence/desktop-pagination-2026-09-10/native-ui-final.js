(async()=>{
const root='/Users/theo/AI 工作流/museion-binarize',invoke=window.__TAURI_INTERNALS__.invoke.bind(window.__TAURI_INTERNALS__);
const report=(stage,data={})=>invoke('plugin:event|emit',{event:'mpdf-ui-test',payload:{stage,at:new Date().toISOString(),...data}});
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(fn,timeout=90000)=>{const t=Date.now();while(Date.now()-t<timeout){if(fn())return;await sleep(40);}throw Error('Timeout '+fn+' '+document.body.innerText.slice(-1200));};
const button=t=>[...document.querySelectorAll('button')].find(b=>b.textContent.trim()===t);
const set=(element,value)=>{const e=typeof element==='string'?document.getElementById(element):element;Object.getOwnPropertyDescriptor(e instanceof HTMLSelectElement?HTMLSelectElement.prototype:HTMLInputElement.prototype,'value').set.call(e,value);e.dispatchEvent(new Event(e instanceof HTMLSelectElement?'change':'input',{bubbles:true}));};
const imageReady=()=>document.querySelector('.page-sheet img')?.complete&&!document.querySelector('.preview-loading');
try {
await until(()=>button('选择 PDF')||button('打开 PDF'));
window.__MPDF_UI_TEST__={open:root+'/docs/evidence/local-bookmarks-2026-09-10/horn-no-outline.pdf',save:root+'/docs/evidence/desktop-pagination-2026-09-10/unused.pdf'};
for(const [name,path] of [['horn','docs/evidence/local-bookmarks-2026-09-10/horn-no-outline.pdf'],['image','docs/evidence/desktop-pagination-2026-09-10/horn-image-only.pdf']]){
 window.__MPDF_UI_TEST__.open=root+'/'+path;
 const began=performance.now();(button('选择 PDF')||button('打开 PDF')).click();await sleep(150);const discard=[...document.querySelectorAll('[role=dialog] button')].find(b=>b.textContent.includes('丢弃'));if(discard)discard.click();
 await until(()=>document.getElementById('current-page')&&document.body.innerText.includes(path.split('/').pop()));await sleep(600);
 await until(()=>/已重建|部分重建|未识别|失败/.test(document.querySelector('.document-analysis-status')?.textContent||''));
 await report(name+'-auto-analysis',{ms:performance.now()-began,status:document.querySelector('.document-analysis-status').textContent});
 if(!document.querySelectorAll('.tool-heading input')[1].checked)document.querySelectorAll('.tool-heading input')[1].click();
 set('contents-pages','6-7');await sleep(600);await until(()=>button('生成')&&!button('生成').disabled);button('生成').click();
 await until(()=>document.querySelectorAll('[role=treeitem]').length>0);await sleep(1000);
 await report(name+'-generated',{count:document.querySelectorAll('[role=treeitem]').length,dom:document.documentElement.outerHTML});
 if(name==='horn'){document.querySelectorAll('.tool-heading input')[1].click();await sleep(100);if(button('丢弃并继续'))button('丢弃并继续').click();}
}
await report('complete');
}catch(e){await report('failure',{error:String(e),dom:document.documentElement.outerHTML});}
})();
