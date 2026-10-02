import {act,fireEvent,render,screen,waitFor} from "@testing-library/react";
import {beforeEach,describe,expect,it,vi} from "vitest";
import App from "./App";
import type {DocumentSummary} from "./app/types";
import type {ContentsResult,LocalEntry} from "./app/localTools";
const mocks=vi.hoisted(()=>({invoke:vi.fn(),listen:vi.fn(),drag:vi.fn(),open:vi.fn(),save:vi.fn()}));
vi.mock("@tauri-apps/api/core",()=>({invoke:(...a:unknown[])=>mocks.invoke(...a)}));
vi.mock("@tauri-apps/api/event",()=>({listen:(...a:unknown[])=>mocks.listen(...a)}));
vi.mock("@tauri-apps/api/window",()=>({getCurrentWindow:()=>({onDragDropEvent:(...a:unknown[])=>mocks.drag(...a)})}));
vi.mock("@tauri-apps/plugin-dialog",()=>({open:(...a:unknown[])=>mocks.open(...a),save:(...a:unknown[])=>mocks.save(...a)}));
vi.mock("@tauri-apps/plugin-opener",()=>({revealItemInDir:vi.fn()}));
const document:DocumentSummary={documentId:"doc-1",fileName:"book.pdf",sourceBytes:123456,pageCount:3,title:null,author:null,pdfiumLibrary:"test",pages:[1,2,3].map(pageNumber=>({pageNumber,widthPoints:595,heightPoints:842,sourceRotationDegrees:0}))};
function sampleEntry(id:string,title:string,target:number|null):LocalEntry{return {id,title,parent:null,level:0,target_pdf_page:target,printed_page:null,source_page:0,source_bbox:{x:30,y:100,width:300,height:12},evidence_ids:[id],review_reasons:["pagination_uninspected"],section_label:null};}
function contents():ContentsResult{const entries=[sampleEntry("entry-0","Introduction",null),sampleEntry("entry-1","Chapter One",2)];return {documentId:"doc-1",sessionId:"contents-1",table:{entries,source_entries:structuredClone(entries),navigation_projection:[],page_count:3},intake:{routes:[{page:1,path:"native_text"}],prepare_seconds:.1,selected_pages:[1],mode:"auto"}};}
async function backend(command:string,args?:Record<string,unknown>):Promise<unknown>{
 if(command==="open_document")return document;
 if(command==="render_preview"){const r=args?.request as {requestId:number;pageNumber:number;kind:string};return {...r,pngBase64:"AA",width:595,height:842,renderDpi:150,isReducedResolution:true};}
 if(command==="local_hierarchy_models")return [{provider:"apple",label:"Apple",state:"unavailable",available:false,message:"modelNotReady"}];
 if(command==="local_bookmark_readiness")return {available:true,imageRecognitionAvailable:true,message:null};
 if(command==="document_analysis_status")return {documentId:"doc-1",textLayer:"present",paginationStatus:"ready",sequenceCount:2,sampledPages:0,message:null};
 if(command==="prepare_local_binarization")return {documentId:"doc-1",preparedId:"prepared-1",pagesProcessed:3,elapsedSeconds:.1};
 if(command==="generate_local_contents")return contents();
 if(command==="save_local_pdf")return {outputPath:"/tmp/output.pdf",pages:3,bookmarksWritten:2,binarized:true,outputBytes:1000,elapsedSeconds:.2};
 if(command==="cancel_local_tools")return;
 throw new Error(`unexpected command ${command}`);
}
beforeEach(()=>{mocks.invoke.mockReset().mockImplementation(backend);mocks.listen.mockReset().mockResolvedValue(()=>{});mocks.drag.mockReset().mockResolvedValue(()=>{});mocks.open.mockReset().mockResolvedValue("/tmp/book.pdf");mocks.save.mockReset().mockResolvedValue("/tmp/output.pdf");});
async function open(){fireEvent.click(screen.getByRole("button",{name:"打开 PDF"}));await screen.findByText("book.pdf");await waitFor(()=>expect(screen.getByRole("button",{name:"开始"})).toBeEnabled());}
async function generate(){fireEvent.click(screen.getByRole("checkbox",{name:/目录书签/}));fireEvent.change(screen.getByLabelText("目录所在的 PDF 页码"),{target:{value:"1"}});await waitFor(()=>expect(screen.getByRole("button",{name:"生成"})).toBeEnabled());fireEvent.click(screen.getByRole("button",{name:"生成"}));await screen.findByRole("tree");}
async function start(){fireEvent.click(screen.getByRole("button",{name:"开始"}));await screen.findByText(/已完成 3 页黑白处理/);}
function accept(){fireEvent.click(screen.getByRole("checkbox",{name:"我已核对目录，按当前内容保存"}));}
describe("unified local workspace",()=>{
 it("opens with one clear workflow and OCR genuinely unavailable",async()=>{render(<App/>);expect(screen.getByRole("button",{name:/OCR 正文识别/})).toBeDisabled();expect(screen.getByRole("button",{name:/保存新 PDF/})).toBeDisabled();await open();expect(mocks.invoke.mock.calls.every(([name])=>(name==="local_ocr_capabilities"||!String(name).includes("ocr"))&&!String(name).includes("api_"))).toBe(true);expect(screen.queryByText("工作目录")).not.toBeInTheDocument();expect(await screen.findByText(/文字层：有/)).toHaveTextContent("已重建");expect(mocks.invoke.mock.calls.some(([name])=>name==="generate_local_contents")).toBe(false);});
 it("opens one native dropped PDF and rejects other drops",async()=>{render(<App/>);await waitFor(()=>expect(mocks.drag).toHaveBeenCalled());const handler=mocks.drag.mock.calls[mocks.drag.mock.calls.length-1][0];act(()=>handler({payload:{type:"drop",paths:["/tmp/a.txt"]}}));expect(await screen.findByRole("alert")).toHaveTextContent("一个 PDF");act(()=>handler({payload:{type:"drop",paths:["/tmp/book.PDF"]}}));await screen.findByText("book.pdf");expect(mocks.open).not.toHaveBeenCalled();});
 it("saves black-and-white alone without requiring a directory or OCR setup",async()=>{render(<App/>);await open();expect(screen.getByRole("button",{name:/保存新 PDF/})).toBeDisabled();await start();fireEvent.click(screen.getByRole("button",{name:/保存新 PDF/}));await screen.findByText("已保存新 PDF");expect(mocks.invoke.mock.calls.filter(([n])=>n==="prepare_local_binarization")).toHaveLength(1);const args=mocks.invoke.mock.calls.find(([name])=>name==="save_local_pdf")![1];expect(args.request).toMatchObject({binarize:true,preparedId:"prepared-1",bookmarkSessionId:null,entries:[],reviewAccepted:false});});
 it("fills targets, edits titles and uses one whole-book confirmation for combined save",async()=>{render(<App/>);await open();await generate();expect(screen.getByRole("button",{name:/保存新 PDF/})).toBeDisabled();expect(screen.getByRole("checkbox",{name:/我已核对/})).toBeDisabled();fireEvent.change(screen.getByLabelText("PDF 目标页"),{target:{value:"2"}});fireEvent.change(screen.getByLabelText("标题"),{target:{value:"Introduction revised"}});accept();await start();fireEvent.click(screen.getByRole("button",{name:/保存新 PDF/}));await screen.findByText("已保存新 PDF");const args=mocks.invoke.mock.calls.find(([name])=>name==="save_local_pdf")![1];expect(args.request).toMatchObject({binarize:true,bookmarkSessionId:"contents-1",reviewAccepted:true,entries:[{id:"entry-0",title:"Introduction revised",parent:null,target_pdf_page:1},{id:"entry-1",title:"Chapter One",parent:null,target_pdf_page:2}]});});
 it("can save bookmarks alone and does not compute processed preview for that mode",async()=>{render(<App/>);await open();fireEvent.click(screen.getByRole("checkbox",{name:/黑白处理/}));await generate();fireEvent.change(screen.getByLabelText("PDF 目标页"),{target:{value:"2"}});accept();expect(screen.queryByRole("radio",{name:"黑白效果"})).not.toBeInTheDocument();fireEvent.click(screen.getByRole("button",{name:/保存新 PDF/}));await screen.findByText("已保存新 PDF");expect(mocks.invoke.mock.calls.find(([n])=>n==="save_local_pdf")![1].request.binarize).toBe(false);});
 it("invalidates review after edits and rejects stale generation settings",async()=>{render(<App/>);await open();await generate();fireEvent.change(screen.getByLabelText("PDF 目标页"),{target:{value:"2"}});accept();fireEvent.change(screen.getByLabelText("标题"),{target:{value:"Changed"}});expect(screen.getByRole("checkbox",{name:/我已核对/})).not.toBeChecked();accept();fireEvent.change(screen.getByLabelText("目录所在的 PDF 页码"),{target:{value:"2"}});expect(screen.getByRole("button",{name:/保存新 PDF/})).toBeDisabled();expect(screen.getByText("目录页或读取方式已改变，请重新生成。")).toBeInTheDocument();});
 it("keeps edits on save failure and prevents duplicate submissions",async()=>{let reject!:(e:unknown)=>void;mocks.invoke.mockImplementation((name:string,args?:Record<string,unknown>)=>name==="save_local_pdf"?new Promise((_,r)=>{reject=r;}):backend(name,args));render(<App/>);await open();await generate();fireEvent.change(screen.getByLabelText("PDF 目标页"),{target:{value:"2"}});accept();await start();fireEvent.click(screen.getByRole("button",{name:/保存新 PDF/}));await screen.findByRole("button",{name:"取消"});expect(screen.getByRole("button",{name:"打开 PDF"})).toBeDisabled();act(()=>reject({code:"local_tools_failed",message:"此位置已有文件",hint:null,detail:null}));await screen.findByText("此位置已有文件");expect(screen.getByLabelText("PDF 目标页")).toHaveValue(2);expect(screen.getByRole("button",{name:/保存新 PDF/})).toBeEnabled();});
 it("cancels with a terminal result and keeps the working document",async()=>{let reject!:(e:unknown)=>void;mocks.invoke.mockImplementation((name:string,args?:Record<string,unknown>)=>{if(name==="save_local_pdf")return new Promise((_,r)=>{reject=r;});if(name==="cancel_local_tools"){reject({code:"cancelled",message:"operation_cancelled",hint:null,detail:null});return Promise.resolve();}return backend(name,args);});render(<App/>);await open();await start();fireEvent.click(screen.getByRole("button",{name:/保存新 PDF/}));fireEvent.click(await screen.findByRole("button",{name:"取消"}));await screen.findByText("已取消，未保存文件。目录修改仍保留。");expect(screen.getByText("book.pdf")).toBeInTheDocument();});
 it("requires a deliberate discard before replacing an edited document",async()=>{render(<App/>);await open();await generate();fireEvent.click(screen.getByRole("button",{name:"打开 PDF"}));await screen.findByRole("dialog",{name:"未保存的修改"});expect(mocks.invoke.mock.calls.filter(([n])=>n==="open_document")).toHaveLength(1);fireEvent.click(screen.getByRole("button",{name:"保留当前内容"}));expect(screen.getByRole("tree")).toBeInTheDocument();});
 it("handles password retry without retaining the submitted password",async()=>{let attempts=0;mocks.invoke.mockImplementation((name:string,args?:Record<string,unknown>)=>{if(name==="open_document"&&attempts++<2)return Promise.reject({code:"password_required",message:"密码错误",hint:null,detail:null});return backend(name,args);});render(<App/>);fireEvent.click(screen.getByRole("button",{name:"打开 PDF"}));await screen.findByRole("dialog",{name:"输入 PDF 密码"});fireEvent.change(screen.getByLabelText("密码"),{target:{value:"wrong"}});fireEvent.click(screen.getByRole("button",{name:"打开"}));await screen.findByText("密码错误");expect(screen.getByLabelText("密码")).toHaveValue("");fireEvent.change(screen.getByLabelText("密码"),{target:{value:"correct"}});fireEvent.click(screen.getByRole("button",{name:"打开"}));await screen.findByText("book.pdf");});
 it("clears irrelevant advanced settings and sends a real manual threshold",async()=>{render(<App/>);await open();fireEvent.change(screen.getByLabelText("黑白算法"),{target:{value:"manual"}});expect(screen.getByLabelText("阈值（0–255）")).toHaveValue(128);expect(screen.queryByLabelText("敏感度 k")).not.toBeInTheDocument();await start();fireEvent.click(screen.getByRole("button",{name:/保存新 PDF/}));await screen.findByText("已保存新 PDF");expect(mocks.invoke.mock.calls.find(([n])=>n==="save_local_pdf")![1].request.settings).toMatchObject({method:"manual",threshold:128,sauvolaK:null,sauvolaWindowSize:null});});
});

describe("selective binarization",()=>{
 it("sends the range minus skipped pages, supports undo and blocks an empty range",async()=>{
  render(<App/>);await open();
  fireEvent.change(screen.getByLabelText("处理范围"),{target:{value:"custom"}});
  fireEvent.change(screen.getByLabelText("黑白处理页码"),{target:{value:"1-2"}});
  fireEvent.click(screen.getByRole("button",{name:"跳过本页"}));
  expect(screen.getByText("处理 1 页，其余 2 页保留原样。")).toBeInTheDocument();
  expect(screen.queryByRole("radio",{name:"黑白效果"})).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button",{name:"恢复本页处理"}));
  fireEvent.click(screen.getByRole("button",{name:"跳过本页"}));
  await start();fireEvent.click(screen.getByRole("button",{name:/保存新 PDF/}));
  await screen.findByText("已保存新 PDF");
  expect(mocks.invoke.mock.calls.find(([n])=>n==="save_local_pdf")![1].request.binarizePages).toEqual([2]);
  fireEvent.change(screen.getByLabelText("黑白处理页码"),{target:{value:"1"}});
  expect(screen.getByRole("button",{name:/保存新 PDF/})).toBeDisabled();
  fireEvent.click(screen.getByRole("button",{name:"仅本页"}));
  expect(screen.getByRole("button",{name:/保存新 PDF/})).toBeDisabled();
  expect(screen.getByText("范围或参数已修改，请再次开始。")).toBeInTheDocument();
 });
 it("does not precompute black-and-white and reuses the original page cache",async()=>{
  render(<App/>);await open();
  await waitFor(()=>expect(mocks.invoke.mock.calls.some(([n,a])=>n==="render_preview"&&a.request.maxDimension===1400)).toBe(true));
  const main=()=>mocks.invoke.mock.calls.filter(([n,a])=>n==="render_preview"&&a.request.maxDimension===1400);
  expect(main().every(([,a])=>a.request.kind==="original"&&a.request.dpi<150)).toBe(true);
  fireEvent.change(screen.getByLabelText("黑白算法"),{target:{value:"manual"}});
  await new Promise(r=>setTimeout(r,240));expect(main()).toHaveLength(1);
  fireEvent.click(screen.getByRole("button",{name:"下一页"}));
  await waitFor(()=>expect(main()).toHaveLength(2));
  fireEvent.click(screen.getByRole("button",{name:"上一页"}));
  await new Promise(r=>setTimeout(r,240));expect(main()).toHaveLength(2);
 });
});

describe("Apple-only hierarchy",()=>{
 it("checks Apple proactively and keeps basic generation available without local-model controls",async()=>{
  render(<App/>);await open();fireEvent.click(screen.getByRole("checkbox",{name:/目录书签/}));
  await screen.findByText(/modelNotReady/);
  expect(mocks.invoke.mock.calls.some(([n])=>n==="local_hierarchy_models")).toBe(true);
  expect(screen.queryByLabelText("目录层级模型")).not.toBeInTheDocument();
  expect(screen.queryByRole("button",{name:"下载模型"})).not.toBeInTheDocument();
  expect(screen.queryByText(/MiniCPM|Qwen/)).not.toBeInTheDocument();
  expect(screen.getByRole("button",{name:"生成"})).toBeEnabled();
  const before=mocks.invoke.mock.calls.filter(([n])=>n==="local_hierarchy_models").length;
  fireEvent.click(screen.getByRole("button",{name:"重新检测"}));
  await waitFor(()=>expect(mocks.invoke.mock.calls.filter(([n])=>n==="local_hierarchy_models")).toHaveLength(before+1));
  expect(mocks.invoke.mock.calls.some(([n])=>n==="download_hierarchy_model")).toBe(false);
 });
 it("always requests Apple and still requires review before saving",async()=>{
  render(<App/>);await open();await generate();
  expect(mocks.invoke.mock.calls.find(([n])=>n==="generate_local_contents")![1].request.hierarchyProvider).toBe("apple");
  expect(screen.getByRole("button",{name:/保存新 PDF/})).toBeDisabled();
 });
});

describe("English localization",()=>{
 it("switches a live document without changing titles, targets or review state, then saves in English",async()=>{
  const view=render(<App/>);await open();await generate();
  fireEvent.change(screen.getByLabelText("PDF 目标页"),{target:{value:"2"}});
  fireEvent.change(screen.getByLabelText("标题"),{target:{value:"目录 — Original title"}});accept();
  fireEvent.change(screen.getByRole("combobox",{name:"Language / 语言"}),{target:{value:"en"}});
  expect(window.document.documentElement.lang).toBe("en");
  expect(localStorage.getItem("museion-binarize.locale")).toBe("en");
  expect(screen.getByLabelText("Title")).toHaveValue("目录 — Original title");
  expect(screen.getByLabelText("PDF target page")).toHaveValue(2);
  expect(screen.getByRole("checkbox",{name:"I have reviewed the contents. Save them as shown."})).toBeChecked();
  expect(screen.getByText(/Text layer: Present/)).toBeInTheDocument();
  expect(screen.getByRole("button",{name:"Check again"})).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button",{name:"Start"}));
  await screen.findByText(/Converted 3 pages to black and white/);
  fireEvent.click(screen.getByRole("button",{name:"Save new PDF…"}));await screen.findByText("New PDF saved");
  expect(mocks.invoke.mock.calls.find(([n])=>n==="save_local_pdf")![1].request.entries[0]).toMatchObject({title:"目录 — Original title",target_pdf_page:1});
  view.unmount();render(<App/>);expect(screen.getByRole("button",{name:"Open PDF"})).toBeInTheDocument();
 });
 it("translates existing errors and validation messages when switching languages",async()=>{
  render(<App/>);await open();
  fireEvent.change(screen.getByLabelText("处理范围"),{target:{value:"custom"}});
  fireEvent.change(screen.getByLabelText("黑白处理页码"),{target:{value:"99"}});
  fireEvent.change(screen.getByRole("combobox",{name:"Language / 语言"}),{target:{value:"en"}});
  expect(screen.getByRole("alert")).toHaveTextContent("Pages to process must be between 1 and 3.");
  expect(screen.getByRole("button",{name:"Start"})).toBeDisabled();
  fireEvent.change(screen.getByRole("combobox",{name:"Language / 语言"}),{target:{value:"zh"}});
  expect(screen.getByRole("alert")).toHaveTextContent("处理页需在 1–3 之间。");
 });
});

it("replaces the whole local OCR panel on a new document without retaining review state",async()=>{
 let opened=0;mocks.invoke.mockImplementation((command:string,args?:Record<string,unknown>)=>{
  if(command==="open_document"){opened++;return Promise.resolve({...document,documentId:`doc-${opened}`,fileName:opened===1?"book.pdf":"next.pdf"});}
  if(command==="local_ocr_capabilities")return Promise.resolve({enabled:true});
  if(command==="mvp_capabilities")return Promise.resolve({enabled:false});
  if(command==="local_ocr_call"){const action=(args?.request as {action:string}).action;return Promise.resolve(action==="readiness"?{local_runtime_ready:true,blockers:[]}:{session_id:"a".repeat(32),revision:1,status:"review_required",source_sha256:"hash",pages:[{page:1,status:"OK",words:[{id:"w",text:"old-source-only",bbox:[1,2,3,4]}]}]});}
  return backend(command,args);
 });
 render(<App/>);await open();await waitFor(()=>expect(screen.getByTestId("local-ocr-start")).toBeEnabled());fireEvent.click(screen.getByTestId("local-ocr-start"));await screen.findByTestId("local-ocr-result");expect(screen.getByText(/old-source-only/)).toBeVisible();fireEvent.click(screen.getByRole("button",{name:"打开 PDF"}));await screen.findByText("next.pdf");await waitFor(()=>expect(screen.queryByTestId("local-ocr-result")).toBeNull());expect(screen.getAllByRole("heading",{name:"本地文字识别"})).toHaveLength(1);expect(screen.queryByText(/old-source-only/)).toBeNull();
});
