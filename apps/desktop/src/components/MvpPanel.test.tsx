import {render,screen,waitFor,fireEvent,within} from "@testing-library/react";
import {describe,it,expect,vi,beforeEach} from "vitest";
import {MvpPanel} from "./MvpPanel";
import {setLocale} from "../lib/i18n";
const ipc=vi.hoisted(()=>vi.fn());
vi.mock("@tauri-apps/api/core",()=>({invoke:ipc}));
vi.mock("@tauri-apps/api/event",()=>({listen:vi.fn().mockResolvedValue(()=>{})}));
vi.mock("../lib/tauri",()=>({pickOutputDestination:vi.fn()}));
const props={documentId:"source",pageCount:12,currentPage:1,previewReady:true,disabled:false,onPage:vi.fn(),onBusy:vi.fn(),onOpen:vi.fn()};

function alternativeResult(){
 const row=(id:string,page:number)=>({alternative_id:id,text:"μακαριώτερον",page,source_sha256:"hash",revision:0,identity_status:"UNIQUE",decision_status:"NOT_RECORDED",review_actions:[],raw_member_id:"tess-w1",stream:"independent_reader",image_sha256:"image",raw_record:{id:"tess-w1",text:"μακαριώτερον"},read_only:true});
 return {session_id:"a".repeat(32),mode:"local",status:"review_required",provenance:"synthetic",revision:0,source_sha256:"hash",pages:[{page:1,status:"OK",words:[{id:"w",text:"adopted",bbox:[1,2,3,4]}]}],reader_alternatives:{source_sha256:"hash",revision:0,read_only:true,rows:[row("raw-page1",1),row("raw-page2",2)],coverage:[{page:1,raw_records:2,adopted_records:1,alternative_records:1,unavailable_streams:[] as string[]},{page:2,raw_records:1,adopted_records:0,alternative_records:1,unavailable_streams:[] as string[]}]}};
}
function installAlternativeIpc(result=alternativeResult()){
 ipc.mockImplementation((command:string,args?:{request:{action:string}})=>Promise.resolve(command==="mvp_capabilities"?{enabled:true}:args?.request.action==="readiness"?{local_runtime_ready:true,blockers:[]}:result));
}
async function importAlternatives(){
 await screen.findByText("OCR 开发诊断");fireEvent.change(screen.getByLabelText("已有结果 JSON 路径"),{target:{value:"/result.json"}});fireEvent.click(screen.getByTestId("mvp-import-button"));await screen.findByTestId("mvp-reader-alternatives");
}
beforeEach(()=>{ipc.mockReset();setLocale("zh");});
describe("page-bound final member selection",()=>{
 it("keeps repeated raw IDs on their selected page and sends the original ID only after viewing that page",async()=>{
  const result={session_id:"a".repeat(32),mode:"local",status:"review_required",provenance:"synthetic",revision:0,source_sha256:"hash",pages:[
   {page:3,status:"OK",words:[{id:"obs-0-w0",text:"182",bbox:[1,2,3,4],source_members:["page3-member"]}]},
   {page:15,status:"OK",words:[{id:"obs-0-w0",text:"Justin",bbox:[5,6,7,8],source_members:["page15-member"]}]},
  ]};
  const original=JSON.stringify(result),onPage=vi.fn();
  ipc.mockImplementation((command:string,args?:{request:{action:string}})=>Promise.resolve(command==="mvp_capabilities"?{enabled:true}:args?.request.action==="readiness"?{local_runtime_ready:true,blockers:[]}:result));
  const view=render(<MvpPanel {...props} pageCount={20} currentPage={3} onPage={onPage}/>);
  await screen.findByText("OCR 开发诊断");fireEvent.change(screen.getByLabelText("已有结果 JSON 路径"),{target:{value:"/synthetic.json"}});fireEvent.click(screen.getByTestId("mvp-import-button"));await screen.findByTestId("mvp-result");
  const first=screen.getByRole("option",{name:"3 · 182 · DRAFT"}) as HTMLOptionElement,last=screen.getByRole("option",{name:"15 · Justin · DRAFT"}) as HTMLOptionElement;
  expect(first.value).not.toBe(last.value);
  fireEvent.change(screen.getByLabelText("位置绑定的待审文字"),{target:{value:first.value}});fireEvent.click(screen.getByTestId("mvp-source"));await waitFor(()=>expect(screen.getByTestId("mvp-accept")).toBeEnabled());
  fireEvent.change(screen.getByLabelText("位置绑定的待审文字"),{target:{value:last.value}});expect(document.getElementById("mvp-text")).toHaveValue("Justin");expect(screen.getByTestId("mvp-accept")).toBeDisabled();
  fireEvent.click(screen.getByTestId("mvp-source"));expect(onPage).toHaveBeenLastCalledWith(15);expect(screen.getByTestId("mvp-accept")).toBeDisabled();fireEvent.click(screen.getByTestId("mvp-accept"));expect(ipc.mock.calls.some(c=>c[1]?.request.action==="review")).toBe(false);
  view.rerender(<MvpPanel {...props} pageCount={20} currentPage={15} onPage={onPage}/>);await waitFor(()=>expect(screen.getByTestId("mvp-accept")).toBeEnabled());fireEvent.click(screen.getByTestId("mvp-accept"));
  await waitFor(()=>expect(ipc.mock.calls.filter(c=>c[1]?.request.action==="review")).toHaveLength(1));
  expect(ipc.mock.calls.find(c=>c[1]?.request.action==="review")?.[1].request).toMatchObject({documentId:"source",mode:"local",expectedRevision:0,actions:[{page:15,member_id:"obs-0-w0",action:"accept",text:"Justin"}]});expect(JSON.stringify(result)).toBe(original);
 });
 it("keeps word and region actions distinct even when their raw ID and page match",async()=>{
  const result={session_id:"a".repeat(32),mode:"local",status:"review_required",provenance:"synthetic",revision:0,source_sha256:"hash",pages:[{page:1,status:"OK",words:[{id:"shared",text:"word",bbox:[1,2,3,4]}],observation:{regions:[{id:"shared",original_text:"paragraph",member_ids:["region-member"],page_number:1,locator:"LOCATED",bbox:[5,6,7,8]}]}}]};
  const original=JSON.stringify(result);
  ipc.mockImplementation((command:string,args?:{request:{action:string}})=>Promise.resolve(command==="mvp_capabilities"?{enabled:true}:args?.request.action==="readiness"?{local_runtime_ready:true,blockers:[]}:result));
  render(<MvpPanel {...props}/>);await screen.findByText("OCR 开发诊断");fireEvent.change(screen.getByLabelText("已有结果 JSON 路径"),{target:{value:"/synthetic.json"}});fireEvent.click(screen.getByTestId("mvp-import-button"));await screen.findByTestId("mvp-result");
  const word=screen.getByRole("option",{name:"1 · word · DRAFT"}) as HTMLOptionElement,region=screen.getByRole("option",{name:/1 · paragraph · LOCATED/}) as HTMLOptionElement;expect(word.value).not.toBe(region.value);
  fireEvent.change(screen.getByLabelText("位置绑定的待审文字"),{target:{value:word.value}});expect(document.getElementById("mvp-text")).toHaveValue("word");expect(screen.queryByText("段落几何，非逐字位置。UNKNOWN 不可采用。")).toBeNull();fireEvent.click(screen.getByTestId("mvp-source"));await waitFor(()=>expect(screen.getByTestId("mvp-accept")).toBeEnabled());fireEvent.click(screen.getByTestId("mvp-accept"));await waitFor(()=>expect(screen.queryByTestId("mvp-accept")).toBeNull());
  expect(ipc.mock.calls.find(c=>c[1]?.request.action==="review")?.[1].request.actions).toEqual([{page:1,member_id:"shared",action:"accept",text:"word"}]);
  fireEvent.change(screen.getByLabelText("位置绑定的待审文字"),{target:{value:region.value}});expect(document.getElementById("mvp-text")).toHaveValue("paragraph");expect(screen.getByText("段落几何，非逐字位置。UNKNOWN 不可采用。")).toBeVisible();expect(screen.getByTestId("mvp-reject")).toBeDisabled();fireEvent.click(screen.getByTestId("mvp-source"));await waitFor(()=>expect(screen.getByTestId("mvp-reject")).toBeEnabled());fireEvent.click(screen.getByTestId("mvp-reject"));
  await waitFor(()=>expect(ipc.mock.calls.filter(c=>c[1]?.request.action==="review")).toHaveLength(2));const action=ipc.mock.calls.filter(c=>c[1]?.request.action==="review")[1][1].request.actions[0];expect(action).toMatchObject({region_id:"shared",member_ids:["region-member"],action:"reject",text:"paragraph",human_approved:false});expect(action).not.toHaveProperty("page");expect(action).not.toHaveProperty("member_id");expect(JSON.stringify(result)).toBe(original);
 });
});
describe("read-only saved reader alternatives",()=>{
 it("clears existing member approval and cannot submit review when viewing raw readings",async()=>{
  installAlternativeIpc();const onPage=vi.fn();render(<MvpPanel {...props} onPage={onPage}/>);await importAlternatives();
  fireEvent.change(screen.getByLabelText("位置绑定的待审文字"),{target:{value:JSON.stringify(["word",1,"w"])}});fireEvent.click(screen.getByTestId("mvp-source"));await waitFor(()=>expect(screen.getByTestId("mvp-accept")).toBeEnabled());
  fireEvent.change(screen.getByLabelText("选择未采用的读法"),{target:{value:"raw-page2"}});expect(screen.queryByTestId("mvp-accept")).toBeNull();
  const detail=screen.getByTestId("mvp-alternative-detail");expect(within(detail).queryByRole("textbox")).toBeNull();expect(within(detail).getAllByRole("button")).toHaveLength(1);
  fireEvent.click(screen.getByTestId("mvp-alternative-source"));expect(onPage).toHaveBeenLastCalledWith(2);
  fireEvent.change(screen.getByLabelText("位置绑定的待审文字"),{target:{value:JSON.stringify(["word",1,"w"])}});expect(screen.getByTestId("mvp-accept")).toBeDisabled();
  expect(screen.queryByTestId("mvp-alternative-detail")).toBeNull();expect(ipc.mock.calls.some(c=>["review","save","start"].includes(c[1]?.request.action))).toBe(false);
 });
 it("keeps same reader IDs on different pages distinct and resets on reload, document and mode",async()=>{
  const result=alternativeResult();installAlternativeIpc(result);const onPage=vi.fn();const view=render(<MvpPanel {...props} onPage={onPage}/>);await importAlternatives();
  fireEvent.change(screen.getByLabelText("选择未采用的读法"),{target:{value:"raw-page1"}});fireEvent.click(screen.getByTestId("mvp-alternative-source"));expect(onPage).toHaveBeenLastCalledWith(1);
  fireEvent.change(screen.getByLabelText("选择未采用的读法"),{target:{value:"raw-page2"}});fireEvent.click(screen.getByTestId("mvp-alternative-source"));expect(onPage).toHaveBeenLastCalledWith(2);
  result.revision=1;result.reader_alternatives.revision=1;result.reader_alternatives.rows.forEach(row=>row.revision=1);
  fireEvent.click(screen.getByTestId("mvp-reload"));await waitFor(()=>expect(screen.queryByTestId("mvp-alternative-detail")).toBeNull());
  fireEvent.change(screen.getByLabelText("选择未采用的读法"),{target:{value:"raw-page2"}});view.rerender(<MvpPanel {...props} documentId="other" onPage={onPage}/>);await waitFor(()=>expect(screen.queryByTestId("mvp-reader-alternatives")).toBeNull());
  await importAlternatives();fireEvent.change(screen.getByLabelText("选择未采用的读法"),{target:{value:"raw-page1"}});fireEvent.change(screen.getByLabelText("识别模式"),{target:{value:"critical-edition"}});await waitFor(()=>expect(screen.queryByTestId("mvp-reader-alternatives")).toBeNull());
 });
 it.each(["revision","source","page-zero","page-outside"])("cannot navigate with invalid %s binding",async(kind)=>{
  const result=alternativeResult(),row=result.reader_alternatives.rows[0];if(kind==="revision")row.revision=3;else if(kind==="source")row.source_sha256="other";else row.page=kind==="page-zero"?0:13;
  installAlternativeIpc(result);const onPage=vi.fn();render(<MvpPanel {...props} onPage={onPage}/>);await importAlternatives();fireEvent.change(screen.getByLabelText("选择未采用的读法"),{target:{value:"raw-page1"}});
  expect(screen.getByTestId("mvp-alternative-source")).toBeDisabled();fireEvent.click(screen.getByTestId("mvp-alternative-source"));expect(onPage).not.toHaveBeenCalled();
 });
 it("keeps missing raw unavailable without recognition",async()=>{
  const result=alternativeResult();result.reader_alternatives.rows=[];result.reader_alternatives.coverage[0].unavailable_streams=["independent_reader"];
  installAlternativeIpc(result);render(<MvpPanel {...props}/>);await importAlternatives();expect(screen.getByText("部分页面没有保留此类原始记录，不会自动重新识别。")).toBeVisible();expect(ipc.mock.calls.some(c=>c[1]?.request.action==="start")).toBe(false);
 });
 it("translates labels while preserving the original reader text",async()=>{
  installAlternativeIpc();setLocale("en");render(<MvpPanel {...props}/>);await screen.findByTestId("mvp-import-button");fireEvent.change(document.getElementById("mvp-import")!,{target:{value:"/result.json"}});fireEvent.click(screen.getByTestId("mvp-import-button"));await screen.findByTestId("mvp-reader-alternatives");
  expect(screen.getByText("Unadopted readings")).toBeVisible();fireEvent.change(screen.getByLabelText("Select an unadopted reading"),{target:{value:"raw-page1"}});expect(within(screen.getByTestId("mvp-alternative-detail")).getByText("μακαριώτερον",{selector:"pre"})).toBeVisible();
 });
});
describe("explicit MVP development boundaries",()=>{
 it("keeps the default release entry hidden",async()=>{ipc.mockResolvedValue({enabled:false});render(<MvpPanel {...props}/>);await waitFor(()=>expect(ipc).toHaveBeenCalled());expect(screen.queryByText("OCR 开发诊断")).toBeNull();});
 it("does not offer a cloud start when paid mode is selected",async()=>{ipc.mockImplementation((command:string)=>Promise.resolve(command==="mvp_capabilities"?{enabled:true}:{local_runtime_ready:true,quality_ready:false,distribution_ready:false,ready:false,blockers:[]}));render(<MvpPanel {...props}/>);await screen.findByText("OCR 开发诊断");fireEvent.change(screen.getByLabelText("识别模式"),{target:{value:"paid"}});await screen.findByText(/云发送已停用/);expect(screen.queryByTestId("mvp-start")).toBeNull();expect(ipc.mock.calls.some(c=>c[1]?.request?.action==="start")).toBe(false);});
 it("shows partial Unicode coverage and requires source viewing before review",async()=>{ipc.mockImplementation((command:string,args?:{request:{action:string}})=>Promise.resolve(command==="mvp_capabilities"?{enabled:true}:args?.request.action==="readiness"?{local_runtime_ready:true,blockers:[]}:{session_id:"a".repeat(32),mode:"local",status:"review_required",provenance:"imported-existing-result",revision:0,export_review:[{member_id:"w",text:"x",issues:["UNSUPPORTED_GLYPH"]}],pages:[{page:1,status:"EXPORT_REVIEW",words:[{id:"w",text:"x",bbox:[1,2,3,4],export_status:"EXPORT_REVIEW"}],export_coverage:{total_words:1,exported_words:0,pending_words:1}}]}));render(<MvpPanel {...props}/>);await screen.findByText("OCR 开发诊断");fireEvent.change(screen.getByLabelText("已有结果 JSON 路径"),{target:{value:"/result.json"}});fireEvent.click(screen.getByTestId("mvp-import-button"));await screen.findByTestId("mvp-export-review");fireEvent.change(screen.getByLabelText("位置绑定的待审文字"),{target:{value:JSON.stringify(["word",1,"w"])}});expect(screen.getByTestId("mvp-accept")).toBeDisabled();fireEvent.click(screen.getByTestId("mvp-source"));expect(screen.getByTestId("mvp-accept")).toBeEnabled();});
});
const tocResult={session_id:"b".repeat(32),mode:"paid-contents",revision:2,status:"review_required",provenance:"existing",table:{revision:2,source_sha256:"source",raw_model_sha256:"raw",page_count:12,entries:[{id:"entry",title:"Title",parent:null,source_page:2,target_pdf_page:5,state:"ready",evidence_ids:["proof"],source_bbox:null}]}};
function tocIpc(command:string,args?:{request:{action:string}}){return Promise.resolve(command==="mvp_capabilities"?{enabled:true}:args?.request.action==="readiness"?{local_runtime_ready:true,blockers:[]}:args?.request.action==="preflight"?{provider:"old-C-manifest"}:tocResult);}
async function importToc(){await screen.findByText("OCR 开发诊断");fireEvent.change(screen.getByLabelText("识别模式"),{target:{value:"paid-contents"}});fireEvent.change(screen.getByLabelText("已有结果 JSON 路径"),{target:{value:"/toc.json"}});fireEvent.click(screen.getByTestId("mvp-import-button"));await screen.findByTestId("mvp-entry-source");}
it("requires a loaded source before a loaded target can authorize review",async()=>{
 ipc.mockImplementation(tocIpc);const view=render(<MvpPanel {...props} previewReady={false}/>);await importToc();fireEvent.click(screen.getByTestId("mvp-entry-source"));fireEvent.click(screen.getByTestId("mvp-entry-target"));view.rerender(<MvpPanel {...props} currentPage={6} previewReady={true}/>);await waitFor(()=>expect(screen.getByTestId("mvp-entry-accept")).toBeDisabled());expect(screen.getByTestId("mvp-entry-reject")).toBeDisabled();
 fireEvent.click(screen.getByTestId("mvp-entry-source"));view.rerender(<MvpPanel {...props} currentPage={3} previewReady={true}/>);await waitFor(()=>expect(screen.getByTestId("mvp-entry-reject")).toBeEnabled());fireEvent.click(screen.getByTestId("mvp-entry-target"));view.rerender(<MvpPanel {...props} currentPage={6} previewReady={true}/>);await waitFor(()=>expect(screen.getByTestId("mvp-entry-accept")).toBeEnabled());fireEvent.click(screen.getByTestId("mvp-partial-confirm"));fireEvent.click(screen.getByTestId("mvp-entry-source"));expect(screen.getByTestId("mvp-partial-confirm")).not.toBeChecked();view.rerender(<MvpPanel {...props} documentId="new-source"/>);await waitFor(()=>expect(screen.queryByTestId("mvp-entry-accept")).toBeNull());
});
it("clears a paid manifest across mode and imported result changes",async()=>{
 ipc.mockImplementation(tocIpc);render(<MvpPanel {...props}/>);await importToc();fireEvent.click(screen.getByTestId("mvp-preflight"));await screen.findByText(/old-C-manifest/);fireEvent.change(screen.getByLabelText("识别模式"),{target:{value:"paid"}});await waitFor(()=>expect(screen.queryByTestId("mvp-preflight-result")).toBeNull());fireEvent.click(screen.getByTestId("mvp-import-button"));await screen.findByTestId("mvp-result");expect(screen.queryByTestId("mvp-preflight-result")).toBeNull();
});

it("invalidates legacy word proof on text edits and leaving the loaded source page",async()=>{
 ipc.mockImplementation((command:string,args?:{request:{action:string}})=>Promise.resolve(command==="mvp_capabilities"?{enabled:true}:args?.request.action==="readiness"?{local_runtime_ready:true,blockers:[]}:{session_id:"a".repeat(32),mode:"local",status:"review_required",provenance:"existing",revision:0,source_sha256:"hash",pages:[{page:1,status:"OK",words:[{id:"w",text:"alpha",bbox:[1,2,3,4],source_members:["member"]}]}]}));
 const view=render(<MvpPanel {...props} previewReady={false}/>);await screen.findByText("OCR 开发诊断");fireEvent.change(screen.getByLabelText("已有结果 JSON 路径"),{target:{value:"/result.json"}});fireEvent.click(screen.getByTestId("mvp-import-button"));await screen.findByTestId("mvp-result");fireEvent.change(screen.getByLabelText("位置绑定的待审文字"),{target:{value:JSON.stringify(["word",1,"w"])}});fireEvent.click(screen.getByTestId("mvp-source"));expect(screen.getByTestId("mvp-accept")).toBeDisabled();view.rerender(<MvpPanel {...props}/>);await waitFor(()=>expect(screen.getByTestId("mvp-accept")).toBeEnabled());fireEvent.change(document.getElementById("mvp-text")!,{target:{value:"beta"}});expect(screen.getByTestId("mvp-accept")).toBeDisabled();fireEvent.change(document.getElementById("mvp-text")!,{target:{value:"alpha"}});expect(screen.getByTestId("mvp-accept")).toBeDisabled();fireEvent.click(screen.getByTestId("mvp-source"));await waitFor(()=>expect(screen.getByTestId("mvp-accept")).toBeEnabled());view.rerender(<MvpPanel {...props} currentPage={2}/>);expect(screen.getByTestId("mvp-accept")).toBeDisabled();view.rerender(<MvpPanel {...props} currentPage={1}/>);expect(screen.getByTestId("mvp-accept")).toBeDisabled();
});
it("keeps selective paid results explicitly diagnostic and cannot submit incompatible review/save",async()=>{
 ipc.mockImplementation((command:string,args?:{request:{action:string}})=>Promise.resolve(command==="mvp_capabilities"?{enabled:true}:args?.request.action==="readiness"?{local_runtime_ready:true,blockers:[]}:{session_id:"a".repeat(32),mode:"paid",status:"review_required",provenance:"selective-v3",revision:0,output_pdf:"/draft.pdf",pages:[{page_number:1,status:"OK",observation:{regions:[{id:"r",original_text:"alpha",member_ids:["m"],page_number:1,locator:"LOCATED",bbox:[1,2,3,4]}]}}]}));
 render(<MvpPanel {...props}/>);await screen.findByText("OCR 开发诊断");fireEvent.change(screen.getByLabelText("识别模式"),{target:{value:"paid"}});await screen.findByTestId("mvp-paid-diagnostic-only");fireEvent.change(screen.getByLabelText("已有结果 JSON 路径"),{target:{value:"/selective.json"}});fireEvent.click(screen.getByTestId("mvp-import-button"));await screen.findByTestId("mvp-result");fireEvent.change(screen.getByLabelText("位置绑定的待审文字"),{target:{value:JSON.stringify(["region",1,"r"])}});fireEvent.click(screen.getByTestId("mvp-source"));await waitFor(()=>expect(screen.getByTestId("mvp-accept")).toBeDisabled());expect(screen.getByTestId("mvp-reject")).toBeDisabled();expect(screen.getByTestId("mvp-change")).toBeDisabled();expect(screen.getByTestId("mvp-save")).toBeDisabled();expect(ipc.mock.calls.some(c=>["review","save"].includes(c[1]?.request.action))).toBe(false);
});
