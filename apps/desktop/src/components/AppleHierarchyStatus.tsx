import { t, localizeMessage } from "../lib/i18n";
import {useEffect,useRef,useState} from "react";
import type {HierarchyModelStatus} from "../app/localTools";
import {localHierarchyModels} from "../lib/tauri";

export function AppleHierarchyStatus({disabled}:{disabled:boolean}) {
  const [model,setModel]=useState<HierarchyModelStatus>();
  const [checking,setChecking]=useState(false),[error,setError]=useState<string|null>(null);
  const mounted=useRef(true),pending=useRef(false);
  async function refresh() {
    if(pending.current)return;
    pending.current=true;setChecking(true);
    try {
      const result=await localHierarchyModels();
      if(mounted.current){setModel(result.find(m=>m.provider==="apple"));setError(null);}
    } catch(e) {
      if(mounted.current)setError(e instanceof Error?e.message:String(e));
    } finally {
      pending.current=false;if(mounted.current)setChecking(false);
    }
  }
  useEffect(()=>{
    mounted.current=true;void refresh();
    const focus=()=>{void refresh();};window.addEventListener("focus",focus);
    return ()=>{mounted.current=false;window.removeEventListener("focus",focus);};
  },[]);
  return <div className="apple-hierarchy-status">
    <strong>{t("Apple 目录层级建议")}</strong>
    <p className="field-hint" role="status">{checking?t("正在检查 Apple 模型…"):model?.available?t("Apple 模型可用"):t("Apple 暂不可用（{0}）。生成时使用基础目录层级。", (model?.message?localizeMessage(model.message):null)??t("尚未完成检测"))}</p>
    <div className="model-actions"><button disabled={disabled||checking} onClick={()=>void refresh()}>{t("重新检测")}</button></div>
    {error&&<p className="attention" role="alert">{localizeMessage(error)}</p>}
    <small className="field-hint">{t("层级建议支持最多 3 页、60 个条目，结果需人工核对。")}</small>
  </div>;
}
