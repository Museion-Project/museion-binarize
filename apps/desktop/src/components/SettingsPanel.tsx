import { t } from "../lib/i18n";
import type {PresetId,ProcessingSettings} from "../app/types";
import {PRESETS,matchingPreset} from "../lib/settings";
interface Props {settings:ProcessingSettings;preset:PresetId;disabled:boolean;onChange:(settings:ProcessingSettings,preset:PresetId)=>void}
const names:Record<PresetId,string>={default:"标准","fine-detail":"小字细节","noisy-scan":"底色不均",custom:"自定义"};
export function SettingsPanel({settings,preset,disabled,onChange}:Props){
  function update(partial:Partial<ProcessingSettings>){const next={...settings,...partial};onChange(next,matchingPreset(next));}
  function method(value:ProcessingSettings["method"]){update({method:value,threshold:value==="manual"?(settings.threshold??128):null,sauvolaWindowSize:value==="sauvola"?settings.sauvolaWindowSize:null,sauvolaK:value==="sauvola"?settings.sauvolaK:null});}
  return <div className="settings-panel"><fieldset disabled={disabled}>
    <label htmlFor="dpi-select">{t("输出分辨率")}</label><select id="dpi-select" value={settings.dpi} onChange={e=>update({dpi:Number(e.target.value)})}>{[300,400,600].map(dpi=><option value={dpi} key={dpi}>{dpi} DPI</option>)}</select>
    <details className="advanced-settings"><summary>{t("高级设置")}{preset==="custom"?` · ${t(names[preset])}`:""}</summary>
    <label htmlFor="preset-select">{t("处理预设")}</label><select id="preset-select" value={preset} onChange={e=>{const id=e.target.value as Exclude<PresetId,"custom">;onChange({...PRESETS[id].settings},id);}}><option value="default">{t("标准")}</option><option value="fine-detail">{t("小字细节")}</option><option value="noisy-scan">{t("底色不均")}</option>{preset==="custom"&&<option value="custom">{t("自定义")}</option>}</select>
    <p className="field-hint">{preset==="fine-detail"?t("提高分辨率，保留细小文字和附加符号。"):preset==="noisy-scan"?t("去除底色与噪点，请检查小字是否完整。"):t("保留细节，先在预览中检查效果。")}</p>

      <label htmlFor="method-select">{t("黑白算法")}</label><select id="method-select" value={settings.method} onChange={e=>method(e.target.value as ProcessingSettings["method"])}><option value="sauvola">{t("局部自适应（Sauvola）")}</option><option value="otsu">{t("全页自动（Otsu）")}</option><option value="manual">{t("手动阈值")}</option></select>
      {settings.method==="manual"&&<><label htmlFor="threshold-input">{t("阈值（0–255）")}</label><input id="threshold-input" type="number" min={0} max={255} value={settings.threshold??128} onChange={e=>update({threshold:Number(e.target.value)})}/></>}
      {settings.method==="sauvola"&&<><label htmlFor="sauvola-window-input">{t("局部窗口（奇数）")}</label><input id="sauvola-window-input" type="number" min={3} step={2} placeholder={t("自动")} value={settings.sauvolaWindowSize??""} onChange={e=>update({sauvolaWindowSize:e.target.value===""?null:Number(e.target.value)})}/><label htmlFor="sauvola-k-input">{t("敏感度 k")}</label><input id="sauvola-k-input" type="number" min={.05} max={.9} step={.05} placeholder="0.20" value={settings.sauvolaK??""} onChange={e=>update({sauvolaK:e.target.value===""?null:Number(e.target.value)})}/></>}
      <label htmlFor="contrast-input">{t("对比度")}{settings.contrast>0?"+":""}{settings.contrast.toFixed(2)}</label><input id="contrast-input" type="range" min={-1} max={1} step={.05} value={settings.contrast} onChange={e=>update({contrast:Number(e.target.value)})}/>
      <label className="check-row"><input type="checkbox" checked={settings.medianDenoise} onChange={e=>update({medianDenoise:e.target.checked})}/>{t("减少噪点")}</label>
      <label className="check-row"><input type="checkbox" checked={settings.backgroundNormalization} onChange={e=>update({backgroundNormalization:e.target.checked,backgroundRadius:e.target.checked?settings.backgroundRadius:null})}/>{t("均衡背景亮度")}</label>
      {settings.backgroundNormalization&&<><label htmlFor="background-radius-input">{t("背景半径（像素）")}</label><input id="background-radius-input" type="number" min={1} placeholder={t("自动")} value={settings.backgroundRadius??""} onChange={e=>update({backgroundRadius:e.target.value===""?null:Number(e.target.value)})}/></>}
      <label htmlFor="despeckle-select">{t("清理细小斑点")}</label><select id="despeckle-select" value={settings.despeckle} onChange={e=>update({despeckle:e.target.value as ProcessingSettings["despeckle"]})}><option value="off">{t("关闭")}</option><option value="conservative">{t("轻度")}</option><option value="strong">{t("强度较高")}</option></select>
    </details>
  </fieldset></div>;
}
