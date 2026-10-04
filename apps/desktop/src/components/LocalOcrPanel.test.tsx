import {render,screen,waitFor,fireEvent,act} from "@testing-library/react";
import {describe,it,expect,vi,beforeEach} from "vitest";
import {LocalOcrPanel} from "./LocalOcrPanel";
import {setLocale} from "../lib/i18n";
const ipc=vi.hoisted(()=>vi.fn());
type Progress={payload:{documentId:string;event:{session_id?:string;client_operation_id?:string;event?:string}}};
const events=vi.hoisted(()=>({handler:null as ((event:Progress)=>void)|null}));
vi.mock("@tauri-apps/api/core",()=>({invoke:ipc}));
vi.mock("@tauri-apps/api/event",()=>({listen:vi.fn().mockImplementation((_event:string,handler:(event:Progress)=>void)=>{events.handler=handler;return Promise.resolve(()=>{});})}));
vi.mock("../lib/tauri",()=>({pickOutputDestination:vi.fn()}));
const props={documentId:"source",pageCount:12,currentPage:1,previewReady:true,disabled:false,onPage:vi.fn(),onBusy:vi.fn(),onOpen:vi.fn()};
const result={session_id:"a".repeat(32),revision:0,status:"review_required",source_sha256:"hash",output_pdf:"/derived.pdf",pages:[{page:1,status:"EXPORT_REVIEW",words:[{id:"word",text:"alpha",bbox:[1,2,3,4],source_members:["member"],export_status:"EXPORT_REVIEW"}]}],export_review:[{member_id:"word",text:"alpha",issues:["UNSUPPORTED_GLYPH"]}]};
beforeEach(()=>{events.handler=null;ipc.mockReset();setLocale("zh");ipc.mockImplementation((c:string,a?:{request:{action:string}})=>Promise.resolve(c==="local_ocr_capabilities"?{enabled:true}:a?.request.action==="readiness"?{local_runtime_ready:true,blockers:[]}:result));});
async function start(){await waitFor(()=>expect(screen.getByTestId("local-ocr-start")).toBeEnabled());fireEvent.click(screen.getByTestId("local-ocr-start"));await screen.findByTestId("local-ocr-result");fireEvent.change(screen.getByLabelText("选择文字核对"),{target:{value:JSON.stringify([1,"word"])}});}
describe("normal local OCR workflow",()=>{
 it("is off in the published build",async()=>{ipc.mockResolvedValue({enabled:false});render(<LocalOcrPanel {...props}/>);await waitFor(()=>expect(ipc).toHaveBeenCalled());expect(screen.queryByText("本地文字识别")).toBeNull();});
 it("offers a plain page selection without developer paths or record inputs",async()=>{render(<LocalOcrPanel {...props}/>);await start();expect(screen.queryByLabelText("已有结果 JSON 路径")).toBeNull();expect(screen.getByTestId("local-ocr-export-review")).toBeVisible();expect(screen.getByTestId("local-ocr-save")).toBeDisabled();});
 it("requires actual original onLoad state and resets proof on text/page/document changes",async()=>{const view=render(<LocalOcrPanel {...props} previewReady={false}/>);await start();fireEvent.click(screen.getByTestId("local-ocr-source"));expect(screen.getByTestId("local-ocr-accept")).toBeDisabled();view.rerender(<LocalOcrPanel {...props} previewReady={true}/>);await waitFor(()=>expect(screen.getByTestId("local-ocr-accept")).toBeEnabled());fireEvent.change(screen.getByLabelText("待审文字"),{target:{value:"beta"}});await waitFor(()=>expect(screen.getByTestId("local-ocr-accept")).toBeDisabled());fireEvent.click(screen.getByTestId("local-ocr-source"));await waitFor(()=>expect(screen.getByTestId("local-ocr-accept")).toBeEnabled());view.rerender(<LocalOcrPanel {...props} currentPage={2}/>);await waitFor(()=>expect(screen.getByTestId("local-ocr-accept")).toBeDisabled());view.rerender(<LocalOcrPanel {...props} documentId="next"/>);await waitFor(()=>expect(screen.queryByTestId("local-ocr-result")).toBeNull());});
 it("discards a stale IPC result after document replacement",async()=>{let resolve:(x:unknown)=>void=()=>{};ipc.mockImplementation((c:string,a?:{request:{action:string}})=>c==="local_ocr_capabilities"?Promise.resolve({enabled:true}):a?.request.action==="start"?new Promise(r=>{resolve=r;}):Promise.resolve({local_runtime_ready:true,blockers:[]}));const view=render(<LocalOcrPanel {...props}/>);await waitFor(()=>expect(screen.getByTestId("local-ocr-start")).toBeEnabled());fireEvent.click(screen.getByTestId("local-ocr-start"));view.rerender(<LocalOcrPanel {...props} documentId="replacement"/>);resolve(result);await waitFor(()=>expect(screen.queryByTestId("local-ocr-cancel")).toBeNull());expect(screen.queryByTestId("local-ocr-result")).toBeNull();});
});

it("defers a pre-session cancellation to the matching operation once and restores review on resume/reload",async()=>{
 let resolve:(x:unknown)=>void=()=>{};let token="";
 ipc.mockImplementation((command:string,args?:{request:{action:string;clientOperationId?:string}})=>{
  if(command==="local_ocr_capabilities")return Promise.resolve({enabled:true});
  if(args?.request.action==="readiness")return Promise.resolve({local_runtime_ready:true,blockers:[]});
  if(args?.request.action==="start"){token=args.request.clientOperationId??"";return new Promise(r=>{resolve=r;});}
  return Promise.resolve(result);
 });
 render(<LocalOcrPanel {...props}/>);await waitFor(()=>expect(screen.getByTestId("local-ocr-start")).toBeEnabled());fireEvent.click(screen.getByTestId("local-ocr-start"));fireEvent.click(screen.getByTestId("local-ocr-cancel"));expect(ipc.mock.calls.filter(c=>c[1]?.request.action==="cancel")).toHaveLength(0);
 await act(async()=>{events.handler?.({payload:{documentId:"source",event:{session_id:"stale",client_operation_id:"old"}}});});expect(ipc.mock.calls.filter(c=>c[1]?.request.action==="cancel")).toHaveLength(0);
 const event={payload:{documentId:"source",event:{session_id:result.session_id,client_operation_id:token}}};await act(async()=>{events.handler?.(event);events.handler?.(event);});const cancellations=ipc.mock.calls.filter(c=>c[1]?.request.action==="cancel");expect(cancellations).toHaveLength(1);expect(cancellations[0][1].request).toMatchObject({sessionId:result.session_id,clientOperationId:token});
 await act(async()=>resolve(result));await screen.findByTestId("local-ocr-result");fireEvent.click(screen.getByTestId("local-ocr-resume"));await waitFor(()=>expect(screen.queryByTestId("local-ocr-cancel")).toBeNull());fireEvent.change(screen.getByLabelText("选择文字核对"),{target:{value:JSON.stringify([1,"word"])}});fireEvent.click(screen.getByTestId("local-ocr-source"));await waitFor(()=>expect(screen.getByTestId("local-ocr-accept")).toBeEnabled());
 fireEvent.click(screen.getByTestId("local-ocr-reload"));await waitFor(()=>expect(screen.queryByTestId("local-ocr-cancel")).toBeNull());fireEvent.change(screen.getByLabelText("选择文字核对"),{target:{value:JSON.stringify([1,"word"])}});fireEvent.click(screen.getByTestId("local-ocr-source"));await waitFor(()=>expect(screen.getByTestId("local-ocr-accept")).toBeEnabled());
});
it("does not deliver an old document's pending cancellation after a document epoch change",async()=>{
 let resolve:(x:unknown)=>void=()=>{};let token="";ipc.mockImplementation((command:string,args?:{request:{action:string;clientOperationId?:string}})=>command==="local_ocr_capabilities"?Promise.resolve({enabled:true}):args?.request.action==="start"?new Promise(r=>{token=args.request.clientOperationId??"";resolve=r;}):Promise.resolve({local_runtime_ready:true,blockers:[]}));
 const view=render(<LocalOcrPanel {...props}/>);await waitFor(()=>expect(screen.getByTestId("local-ocr-start")).toBeEnabled());fireEvent.click(screen.getByTestId("local-ocr-start"));fireEvent.click(screen.getByTestId("local-ocr-cancel"));const oldHandler=events.handler;view.rerender(<LocalOcrPanel {...props} documentId="new"/>);await act(async()=>oldHandler?.({payload:{documentId:"source",event:{session_id:result.session_id,client_operation_id:token}}}));expect(ipc.mock.calls.some(c=>c[1]?.request.action==="cancel")).toBe(false);await act(async()=>resolve(result));expect(screen.queryByTestId("local-ocr-result")).toBeNull();
});

const pendingSave={journal_id:`save-pending-${"b".repeat(32)}.json`,journal_sha256:"c".repeat(64),status:"complete_copy_pending_receipt",recoverable:true,output_pdf:"/existing-complete.pdf",reason:null};
const continuable={...result,output_pdf:undefined,revision:null,status:"processing_unverified",draft_published:false,runtime_compatible:true,continuation_available:true,continuation_state_sha256:"f".repeat(64),pages:[...result.pages,{page:2,status:"NOT_PROCESSED"}]};
it("views an interrupted draft without processing until the separate continue action",async()=>{
 ipc.mockImplementation((command:string,args?:{request:{action:string}})=>Promise.resolve(command==="local_ocr_capabilities"?{enabled:true}:args?.request.action==="readiness"?{local_runtime_ready:true,blockers:[]}:args?.request.action==="continue"?result:continuable));
 render(<LocalOcrPanel {...props}/>);await waitFor(()=>expect(screen.getByTestId("local-ocr-resume")).toBeEnabled());expect(screen.getByTestId("local-ocr-resume")).toHaveTextContent("查看上次本地草稿");fireEvent.click(screen.getByTestId("local-ocr-resume"));await screen.findByTestId("local-ocr-continue");
 expect(ipc.mock.calls.some(c=>c[1]?.request.action==="continue"||c[1]?.request.action==="start")).toBe(false);
 fireEvent.change(screen.getByLabelText("识别 PDF 页码"),{target:{value:"9-10"}});fireEvent.click(screen.getByTestId("local-ocr-continue"));await waitFor(()=>expect(screen.queryByTestId("local-ocr-continue")).toBeNull());
 const request=ipc.mock.calls.find(c=>c[1]?.request.action==="continue")?.[1].request;
 expect(request).toMatchObject({documentId:"source",mode:"local",sessionId:result.session_id,continuationStateSha256:continuable.continuation_state_sha256});expect(request.clientOperationId).toMatch(/^[0-9a-f]{32}$/);expect(request.pages).toBeUndefined();expect(ipc.mock.calls.some(c=>c[1]?.request.action==="start")).toBe(false);
});
it("queues continuation cancel until the new token is admitted and rejects old token or wrong session events",async()=>{
 let resolve:(x:unknown)=>void=()=>{};let token="";ipc.mockImplementation((command:string,args?:{request:{action:string;clientOperationId?:string}})=>command==="local_ocr_capabilities"?Promise.resolve({enabled:true}):args?.request.action==="readiness"?Promise.resolve({local_runtime_ready:true,blockers:[]}):args?.request.action==="continue"?new Promise(r=>{token=args.request.clientOperationId??"";resolve=r;}):Promise.resolve(continuable));
 render(<LocalOcrPanel {...props}/>);await waitFor(()=>expect(screen.getByTestId("local-ocr-resume")).toBeEnabled());fireEvent.click(screen.getByTestId("local-ocr-resume"));await screen.findByTestId("local-ocr-continue");fireEvent.click(screen.getByTestId("local-ocr-continue"));fireEvent.click(screen.getByTestId("local-ocr-cancel"));expect(ipc.mock.calls.some(c=>c[1]?.request.action==="cancel")).toBe(false);
 await act(async()=>events.handler?.({payload:{documentId:"source",event:{session_id:result.session_id,client_operation_id:"c".repeat(32)}}}));expect(ipc.mock.calls.some(c=>c[1]?.request.action==="cancel")).toBe(false);
 await act(async()=>events.handler?.({payload:{documentId:"source",event:{session_id:"wrong",client_operation_id:token}}}));expect(ipc.mock.calls.some(c=>c[1]?.request.action==="cancel")).toBe(false);
 const admitted={payload:{documentId:"source",event:{session_id:result.session_id,client_operation_id:token}}};await act(async()=>{events.handler?.(admitted);events.handler?.(admitted);events.handler?.({payload:{documentId:"source",event:{session_id:"wrong",client_operation_id:token}}});});
 const cancel=ipc.mock.calls.filter(c=>c[1]?.request.action==="cancel");expect(cancel).toHaveLength(1);expect(cancel[0][1].request).toMatchObject({sessionId:result.session_id,clientOperationId:token});await act(async()=>resolve({...result,status:"cancelled"}));
});
it("does not continue an unverified, incompatible, completed or unavailable draft",async()=>{
 const stale={...continuable,runtime_compatible:false};ipc.mockImplementation((command:string,args?:{request:{action:string}})=>Promise.resolve(command==="local_ocr_capabilities"?{enabled:true}:args?.request.action==="readiness"?{local_runtime_ready:true,blockers:[]}:stale));
 render(<LocalOcrPanel {...props}/>);await waitFor(()=>expect(screen.getByTestId("local-ocr-resume")).toBeEnabled());fireEvent.click(screen.getByTestId("local-ocr-resume"));await screen.findByTestId("local-ocr-continue");expect(screen.getByTestId("local-ocr-continue")).toBeDisabled();
 ipc.mockImplementation((command:string,args?:{request:{action:string}})=>Promise.resolve(command==="local_ocr_capabilities"?{enabled:true}:args?.request.action==="readiness"?{local_runtime_ready:true,blockers:[]}:({...continuable,continuation_available:false})));fireEvent.click(screen.getByTestId("local-ocr-reload"));await waitFor(()=>expect(screen.queryByTestId("local-ocr-continue")).toBeNull());expect(ipc.mock.calls.some(c=>c[1]?.request.action==="continue")).toBe(false);
});
it("discards continuation results and pending cancellation after document replacement",async()=>{
 let resolve:(x:unknown)=>void=()=>{};let token="";ipc.mockImplementation((command:string,args?:{request:{action:string;clientOperationId?:string}})=>command==="local_ocr_capabilities"?Promise.resolve({enabled:true}):args?.request.action==="readiness"?Promise.resolve({local_runtime_ready:true,blockers:[]}):args?.request.action==="continue"?new Promise(r=>{token=args.request.clientOperationId??"";resolve=r;}):Promise.resolve(continuable));
 const view=render(<LocalOcrPanel {...props}/>);await waitFor(()=>expect(screen.getByTestId("local-ocr-resume")).toBeEnabled());fireEvent.click(screen.getByTestId("local-ocr-resume"));await screen.findByTestId("local-ocr-continue");fireEvent.click(screen.getByTestId("local-ocr-continue"));fireEvent.click(screen.getByTestId("local-ocr-cancel"));const handler=events.handler;view.rerender(<LocalOcrPanel {...props} documentId="replacement"/>);
 await act(async()=>handler?.({payload:{documentId:"source",event:{session_id:result.session_id,client_operation_id:token}}}));expect(ipc.mock.calls.some(c=>c[1]?.request.action==="cancel")).toBe(false);await act(async()=>resolve(result));expect(screen.queryByTestId("local-ocr-result")).toBeNull();
});
it.each([true,false])("keeps unpublished partial results read-only with runtime compatibility %s",async(compatible)=>{
 const pending={...result,output_pdf:undefined,revision:null,status:"processing_unverified",draft_published:false,runtime_compatible:compatible,completion_record_state:"not_published",pages:[...result.pages,{page:2,status:"NATIVE_PRESERVED",native_text:"saved native text"},{page:3,status:"NOT_PROCESSED"}]};
 ipc.mockImplementation((command:string,args?:{request:{action:string}})=>Promise.resolve(command==="local_ocr_capabilities"?{enabled:true}:args?.request.action==="readiness"?{local_runtime_ready:true,blockers:[]}:pending));
 render(<LocalOcrPanel {...props}/>);await waitFor(()=>expect(screen.getByTestId("local-ocr-resume")).toBeEnabled());fireEvent.click(screen.getByTestId("local-ocr-resume"));await screen.findByTestId("local-ocr-incomplete");
 expect(screen.getByText(/部分结果（仅查看）/)).toBeVisible();expect(screen.queryByText(/修订 null/)).toBeNull();expect(screen.getByText("saved native text")).toBeInTheDocument();
 fireEvent.change(screen.getByLabelText("选择文字核对"),{target:{value:JSON.stringify([1,"word"])}});fireEvent.click(screen.getByTestId("local-ocr-source"));fireEvent.click(screen.getByTestId("local-ocr-partial"));
 expect(screen.getByTestId("local-ocr-accept")).toBeDisabled();expect(screen.getByTestId("local-ocr-reject")).toBeDisabled();expect(screen.getByTestId("local-ocr-change")).toBeDisabled();expect(screen.getByTestId("local-ocr-save")).toBeDisabled();expect(screen.queryByTestId("local-ocr-search")).toBeNull();
 expect(Boolean(screen.queryByTestId("local-ocr-runtime-stale"))).toBe(!compatible);
 fireEvent.click(screen.getByTestId("local-ocr-reload"));await screen.findByTestId("local-ocr-incomplete");
 expect(ipc.mock.calls.some(c=>["start","review","save","recover-save"].includes(c[1]?.request.action))).toBe(false);
});
it.each(["missing","unreadable"])("shows a %s completion record on resume without starting recognition",async(completionState)=>{
 const pending={...result,status:"completion_unverified",completion_record_state:completionState,runtime_compatible:true};
 ipc.mockImplementation((command:string,args?:{request:{action:string}})=>Promise.resolve(command==="local_ocr_capabilities"?{enabled:true}:args?.request.action==="readiness"?{local_runtime_ready:true,blockers:[]}:pending));
 render(<LocalOcrPanel {...props}/>);await waitFor(()=>expect(screen.getByTestId("local-ocr-resume")).toBeEnabled());
 fireEvent.click(screen.getByTestId("local-ocr-resume"));await screen.findByTestId("local-ocr-completion-unverified");
 expect(screen.getByText(/处理完成情况待确认/)).toBeVisible();expect(screen.queryByText(/completion_unverified/)).toBeNull();
 expect(screen.getByText("草稿已保留，但处理完成记录缺失或无法读取。请先核对结果；不会自动重跑。")).toBeVisible();
 expect(screen.getByTestId("local-ocr-save")).toBeDisabled();expect(screen.getByTestId("local-ocr-reload")).toBeEnabled();
 expect(ipc.mock.calls.some(c=>c[1]?.request.action==="start")).toBe(false);
});
it("requires draft acknowledgement and sends the exact journal/revision for receipt recovery",async()=>{
 const pending={...result,runtime_compatible:true,session_storage:"persistent-private",pending_saves:[pendingSave]};
 ipc.mockImplementation((command:string,args?:{request:{action:string}})=>Promise.resolve(command==="local_ocr_capabilities"?{enabled:true}:args?.request.action==="readiness"?{local_runtime_ready:true,blockers:[]}:args?.request.action==="recover-save"?{...pending,saved:{output_pdf:pendingSave.output_pdf},pending_saves:[{...pendingSave,status:"recovered",recoverable:false}]}:pending));
 render(<LocalOcrPanel {...props}/>);await start();expect(screen.getByTestId("local-ocr-recover-save")).toBeDisabled();
 fireEvent.click(screen.getByTestId("local-ocr-partial"));expect(screen.getByTestId("local-ocr-recover-save")).toBeEnabled();
 fireEvent.click(screen.getByTestId("local-ocr-recover-save"));await screen.findByTestId("local-ocr-saved");
 const request=ipc.mock.calls.find(c=>c[1]?.request.action==="recover-save")?.[1].request;
 expect(request).toMatchObject({documentId:"source",mode:"local",sessionId:result.session_id,expectedRevision:0,saveJournalId:pendingSave.journal_id,saveJournalSha256:pendingSave.journal_sha256});
 expect(ipc.mock.calls.filter(c=>c[1]?.request.action==="start")).toHaveLength(1);expect(ipc.mock.calls.filter(c=>c[1]?.request.action==="save")).toHaveLength(0);
 expect(screen.getByText("保存收据已恢复，PDF 内容未改动。")).toBeVisible();expect(screen.queryByTestId("local-ocr-recover-save")).toBeNull();
});
it("keeps a runtime-mismatched legacy draft readable with mutation buttons disabled",async()=>{
 const stale={...result,runtime_compatible:false,session_storage:"legacy-temporary",pending_saves:[pendingSave]};
 ipc.mockImplementation((command:string,args?:{request:{action:string}})=>Promise.resolve(command==="local_ocr_capabilities"?{enabled:true}:args?.request.action==="readiness"?{local_runtime_ready:true,blockers:[]}:stale));
 render(<LocalOcrPanel {...props}/>);await start();fireEvent.click(screen.getByTestId("local-ocr-source"));fireEvent.click(screen.getByTestId("local-ocr-partial"));
 expect(screen.getByTestId("local-ocr-runtime-stale")).toBeVisible();expect(screen.getByTestId("local-ocr-accept")).toBeDisabled();expect(screen.getByTestId("local-ocr-change")).toBeDisabled();expect(screen.getByTestId("local-ocr-save")).toBeDisabled();expect(screen.getByTestId("local-ocr-recover-save")).toBeDisabled();expect(screen.getByTestId("local-ocr-reload")).toBeEnabled();
});
it("keeps unresolved interrupted saves visible without offering receipt recovery",async()=>{
 const pending={...result,runtime_compatible:true,pending_saves:[{...pendingSave,recoverable:false,status:"unresolved",reason:"RECOVERY_OUTPUT_CHANGED_OR_REPLACED"}]};
 ipc.mockImplementation((command:string,args?:{request:{action:string}})=>Promise.resolve(command==="local_ocr_capabilities"?{enabled:true}:args?.request.action==="readiness"?{local_runtime_ready:true,blockers:[]}:pending));
 render(<LocalOcrPanel {...props}/>);await start();expect(screen.getByText("无法确认这次保存，请保留现有文件。可以另选新文件名保存。")).toBeVisible();expect(screen.queryByTestId("local-ocr-recover-save")).toBeNull();
});
it("discards a recovery result after the source document changes",async()=>{
 let resolve:(x:unknown)=>void=()=>{};const pending={...result,runtime_compatible:true,pending_saves:[pendingSave]};
 ipc.mockImplementation((command:string,args?:{request:{action:string}})=>command==="local_ocr_capabilities"?Promise.resolve({enabled:true}):args?.request.action==="readiness"?Promise.resolve({local_runtime_ready:true,blockers:[]}):args?.request.action==="recover-save"?new Promise(r=>{resolve=r;}):Promise.resolve(pending));
 const view=render(<LocalOcrPanel {...props}/>);await start();fireEvent.click(screen.getByTestId("local-ocr-partial"));fireEvent.click(screen.getByTestId("local-ocr-recover-save"));view.rerender(<LocalOcrPanel {...props} documentId="replacement"/>);
 await act(async()=>resolve({...pending,saved:{output_pdf:pendingSave.output_pdf}}));expect(screen.queryByTestId("local-ocr-result")).toBeNull();expect(screen.queryByTestId("local-ocr-saved")).toBeNull();
});

it("binds repeated member IDs to the selected physical page and preserves the backend ID",async()=>{
 const repeated={...result,pages:[{page:1,status:"OCR_DRAFT",words:[{id:"word",text:"alpha",bbox:[1,2,3,4],source_members:["raw-alpha"]}]},{page:2,status:"OCR_DRAFT",words:[{id:"word",text:"beta",bbox:[5,6,7,8],source_members:["raw-beta"]}]}]};
 ipc.mockImplementation((command:string,args?:{request:{action:string}})=>Promise.resolve(command==="local_ocr_capabilities"?{enabled:true}:args?.request.action==="readiness"?{local_runtime_ready:true,blockers:[]}:repeated));
 render(<LocalOcrPanel {...props} currentPage={2}/>);await waitFor(()=>expect(screen.getByTestId("local-ocr-resume")).toBeEnabled());fireEvent.click(screen.getByTestId("local-ocr-resume"));await screen.findByTestId("local-ocr-result");
 const target=screen.getByRole("option",{name:"第 2 页 · beta · DRAFT"}) as HTMLOptionElement;fireEvent.change(screen.getByLabelText("选择文字核对"),{target:{value:target.value}});fireEvent.click(screen.getByTestId("local-ocr-source"));
 expect(screen.getByLabelText("待审文字")).toHaveValue("beta");expect(props.onPage).toHaveBeenLastCalledWith(2);await waitFor(()=>expect(screen.getByTestId("local-ocr-accept")).toBeEnabled());fireEvent.click(screen.getByTestId("local-ocr-accept"));
 await waitFor(()=>expect(ipc.mock.calls.some(c=>c[1]?.request.action==="review")).toBe(true));const request=ipc.mock.calls.find(c=>c[1]?.request.action==="review")?.[1].request;
 expect(request).toMatchObject({documentId:"source",mode:"local",sessionId:result.session_id,expectedRevision:0,actions:[{page:2,member_id:"word",action:"accept",text:"beta"}]});
});

it("clears source proof when selecting the same raw ID on another page",async()=>{
 const repeated={...result,pages:[{page:1,status:"OCR_DRAFT",words:[{id:"word",text:"alpha",bbox:[1,2,3,4]}]},{page:2,status:"OCR_DRAFT",words:[{id:"word",text:"beta",bbox:[5,6,7,8]}]}]};
 ipc.mockImplementation((command:string,args?:{request:{action:string}})=>Promise.resolve(command==="local_ocr_capabilities"?{enabled:true}:args?.request.action==="readiness"?{local_runtime_ready:true,blockers:[]}:repeated));
 const view=render(<LocalOcrPanel {...props}/>);await waitFor(()=>expect(screen.getByTestId("local-ocr-resume")).toBeEnabled());fireEvent.click(screen.getByTestId("local-ocr-resume"));await screen.findByTestId("local-ocr-result");
 const select=screen.getByLabelText("选择文字核对"),alpha=screen.getByRole("option",{name:"第 1 页 · alpha · DRAFT"}) as HTMLOptionElement,beta=screen.getByRole("option",{name:"第 2 页 · beta · DRAFT"}) as HTMLOptionElement;
 fireEvent.change(select,{target:{value:alpha.value}});fireEvent.click(screen.getByTestId("local-ocr-source"));await waitFor(()=>expect(screen.getByTestId("local-ocr-accept")).toBeEnabled());
 fireEvent.change(select,{target:{value:beta.value}});await waitFor(()=>expect(screen.getByTestId("local-ocr-accept")).toBeDisabled());expect(screen.getByLabelText("待审文字")).toHaveValue("beta");fireEvent.click(screen.getByTestId("local-ocr-source"));expect(props.onPage).toHaveBeenLastCalledWith(2);expect(screen.getByTestId("local-ocr-accept")).toBeDisabled();view.rerender(<LocalOcrPanel {...props} currentPage={2}/>);await waitFor(()=>expect(screen.getByTestId("local-ocr-accept")).toBeEnabled());
});
it("keeps delimiter-bearing original IDs intact with distinct page selection values",async()=>{
 const repeated={...result,pages:[{page:1,status:"OCR_DRAFT",words:[{id:'2:x',text:"alpha",bbox:[1,2,3,4]}]},{page:2,status:"OCR_DRAFT",words:[{id:'x',text:"beta",bbox:[5,6,7,8]}]},{page:3,status:"OCR_DRAFT",words:[{id:'[2,"x"]',text:"gamma",bbox:[9,10,11,12]}]}]};
 ipc.mockImplementation((command:string,args?:{request:{action:string}})=>Promise.resolve(command==="local_ocr_capabilities"?{enabled:true}:args?.request.action==="readiness"?{local_runtime_ready:true,blockers:[]}:repeated));render(<LocalOcrPanel {...props}/>);await waitFor(()=>expect(screen.getByTestId("local-ocr-resume")).toBeEnabled());fireEvent.click(screen.getByTestId("local-ocr-resume"));await screen.findByTestId("local-ocr-result");
 const select=screen.getByLabelText("选择文字核对") as HTMLSelectElement,options=Array.from(select.options).filter(x=>x.value);expect(new Set(options.map(x=>x.value)).size).toBe(3);
 for(const [index,text] of ['alpha','beta','gamma'].entries()){fireEvent.change(select,{target:{value:options[index].value}});expect(screen.getByLabelText("待审文字")).toHaveValue(text);}
 expect(repeated.pages[2].words[0].id).toBe('[2,"x"]');expect(ipc.mock.calls.some(c=>c[1]?.request.action==="review")).toBe(false);
});
