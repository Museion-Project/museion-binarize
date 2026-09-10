import {describe,it,expect} from "vitest";
import {parseContentsPages,orderedTree,descendants,changeProjection,type LocalEntry,type ContentsDraft} from "./localTools";
const entry=(id:string,parent:string|null=null):LocalEntry=>({id,parent,title:id,level:0,target_pdf_page:1,printed_page:"1",source_page:0,source_bbox:{x:0,y:0,width:10,height:10},evidence_ids:[],review_reasons:[],section_label:null});
describe("local workflow contracts",()=>{
 it("parses physical page ranges without silently ignoring mistakes",()=>{expect(parseContentsPages("6-7, 13，6",400).pages).toEqual([6,7,13]);for(const s of ["0","3-1","1-402","abc","1-50"])expect(parseContentsPages(s,400).error).not.toBeNull();});
 it("rejects cycles, missing parents and duplicate IDs",()=>{expect(()=>orderedTree([entry("a","b"),entry("b","a")])).toThrow();expect(()=>orderedTree([entry("a","x")])).toThrow();expect(()=>orderedTree([entry("a"),entry("a")])).toThrow();});
 it("keeps descendants adjacent when reparenting",()=>{expect(orderedTree([entry("a"),entry("b"),entry("c","a")]).map(e=>[e.id,e.level])).toEqual([["a",0],["c",1],["b",0]]);expect([...descendants([entry("a"),entry("b","a"),entry("c","b")],"a")]).toEqual(["a","b","c"]);});
 it("restores source containers without erasing user titles, targets or deletions",()=>{
  const source=[entry("container"),entry("a","container"),entry("b","container")],nav=[entry("a"),entry("b")];
  const draft={entries:[{...entry("a"),title:"Edited",target_pdf_page:40}],result:{table:{entries:nav,source_entries:source,navigation_projection:[{source_id:"container",requires_review:true}]}},projection:"navigation"} as ContentsDraft;
  const restored=changeProjection(draft,"source");expect(restored.map(e=>e.id)).toEqual(["container","a"]);expect(restored[1]).toMatchObject({title:"Edited",target_pdf_page:40,parent:"container"});
  expect(changeProjection({...draft,entries:restored,projection:"source"},"navigation")[0]).toMatchObject({id:"a",title:"Edited",target_pdf_page:40,parent:null});
 });
});
