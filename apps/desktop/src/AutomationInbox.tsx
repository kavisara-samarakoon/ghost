import type { DesktopPage } from "./DesktopPages.tsx";
import { openAutomationItem,taskText,type AutomationItem,type CommandPrefill } from "./automations.ts";
export default function AutomationInbox({items,onOpen,onHandle,busy=false}:{items:AutomationItem[];onOpen?:(page:DesktopPage,prefill:CommandPrefill|null,alias:string|null)=>void;onHandle?:(id:string,status:"acknowledged"|"dismissed")=>void;busy?:boolean}) {
  return <section className="surface"><h2>Due now</h2>{items.length===0?<p>No pending automation attention items.</p>:items.map(item=><article className="automation-row" key={item.id}>
    <h3>{item.automation_title}</h3><p className="automation-text">{taskText(item.task)}</p><p>Automation created this reminder only. No action has run.</p>
    <div className="button-row">{item.task.kind!=="reminder"&&onOpen&&<button disabled={busy} onClick={()=>openAutomationItem(item,onOpen)}>{item.task.kind==="command_prompt"?"Open in Command":"Open page"}</button>}
      <button disabled={busy||!onHandle} onClick={()=>onHandle?.(item.id,"acknowledged")}>Acknowledge</button><button disabled={busy||!onHandle} onClick={()=>onHandle?.(item.id,"dismissed")}>Dismiss</button></div>
    <details><summary>Advanced details</summary><p>Item {item.id} · Automation {item.automation_id}</p><p>{item.occurrence_key} · {new Date(item.triggered_at*1000).toISOString()}</p></details>
  </article>)}</section>;
}
