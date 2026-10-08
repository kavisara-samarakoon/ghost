import { useState } from "react";
import type { GhostProject } from "./ghost-snapshot.ts";
import { automationPages,weekdays,type AutomationDefinition,type AutomationPayload,type Trigger,type Task } from "./automations.ts";
export default function AutomationForm({definition,projects,busy,onChange,onPrepare}:{definition?:AutomationDefinition;projects:GhostProject[];busy:boolean;onChange:()=>void;onPrepare:(p:AutomationPayload)=>void}) {
  const original=definition?.payload;const [title,setTitle]=useState(original?.title??"");const [kind,setKind]=useState<Trigger["kind"]>(original?.trigger.kind??"once");
  const recurring=original?.trigger.kind==="daily"||original?.trigger.kind==="weekly"?original.trigger:null;
  const [once,setOnce]=useState(original?.trigger.kind==="once"?original.trigger.at:"");const [time,setTime]=useState(recurring?String(recurring.hour).padStart(2,"0")+":"+String(recurring.minute).padStart(2,"0"):"09:00");
  const [offset,setOffset]=useState(recurring?.offset_minutes??-new Date().getTimezoneOffset());const [weekday,setWeekday]=useState<typeof weekdays[number]>(original?.trigger.kind==="weekly"?original.trigger.weekday:"monday");
  const [alias,setAlias]=useState(original?.trigger.kind==="project_no_active_session"?original.trigger.project_alias:projects[0]?.alias??"");
  const [taskKind,setTaskKind]=useState<Task["kind"]>(original?.task.kind??"reminder");const [text,setText]=useState(original?.task.kind==="command_prompt"?original.task.prompt:original?.task.message??"");
  const [page,setPage]=useState<Lowercase<typeof automationPages[number]>>(original?.task.kind==="review_page"?original.task.page:"today");const [binding,setBinding]=useState(original?.task.kind==="command_prompt"?original.task.project_alias??"":"");
  const [error,setError]=useState("");
  function prepare(){
    const [hour,minute]=time.split(":").map(Number);let trigger:Trigger;
    if(kind==="once"){if(Number.isNaN(Date.parse(once))||!/(Z|[+-]\d\d:\d\d)$/.test(once)){setError("Enter an RFC3339 instant with Z or an explicit offset.");return;}trigger={kind,at:once};}
    else if(kind==="daily")trigger={kind,hour,minute,offset_minutes:offset};
    else if(kind==="weekly")trigger={kind,weekday,hour,minute,offset_minutes:offset};
    else if(kind==="project_no_active_session")trigger={kind,project_alias:alias};else trigger={kind};
    const task:Task=taskKind==="command_prompt"?{kind:taskKind,prompt:text,project_alias:binding||null}:taskKind==="review_page"?{kind:taskKind,page,message:text}:{kind:taskKind,message:text};
    setError("");onPrepare({title,enabled:original?.enabled??true,trigger,task});
  }
  return <form className="surface automation-form" onSubmit={e=>{e.preventDefault();prepare();}} onChange={onChange}><h2>{definition?"Edit automation":"New automation"}</h2><fieldset disabled={busy}>
    <label>Title<input value={title} onChange={e=>setTitle(e.target.value)} maxLength={160} required/></label>
    <label>Trigger<select value={kind} onChange={e=>setKind(e.target.value as Trigger["kind"])}><option value="once">Once</option><option value="daily">Daily</option><option value="weekly">Weekly</option><option value="project_no_active_session" disabled={!projects.length}>Local condition: project has no active session</option><option value="recent_pending_requests_present">Local condition: recent pending requests (global)</option></select></label>
    {kind==="once"?<label>Exact RFC3339 instant (UTC or explicit offset)<input value={once} onChange={e=>setOnce(e.target.value)} placeholder="2027-01-01T09:00:00+05:30" maxLength={40} required/></label>:kind==="daily"||kind==="weekly"?<><label>Wall-clock time<input type="time" value={time} onChange={e=>setTime(e.target.value)} required/></label><label>Fixed UTC offset in minutes<input type="number" value={offset} min={-840} max={840} step={1} onChange={e=>setOffset(Number(e.target.value))}/></label>{kind==="weekly"&&<label>Weekday<select value={weekday} onChange={e=>setWeekday(e.target.value as typeof weekdays[number])}>{weekdays.map(day=><option key={day}>{day}</option>)}</select></label>}</>:kind==="project_no_active_session"?<label>Registered project<select value={alias} onChange={e=>setAlias(e.target.value)} required>{projects.map(p=><option key={p.alias} value={p.alias}>{p.name}</option>)}</select></label>:<p>Uses the existing bounded recent pending-request reader, globally. No provider reads.</p>}
    <label>Task<select value={taskKind} onChange={e=>setTaskKind(e.target.value as Task["kind"])}><option value="reminder">Reminder</option><option value="command_prompt">Ask GHOST prompt</option><option value="review_page">Review page</option></select></label>
    <label>{taskKind==="command_prompt"?"Prompt":"Message"}<textarea value={text} maxLength={8192} rows={4} onChange={e=>setText(e.target.value)} required/></label>
    {taskKind==="command_prompt"&&<label>Optional registered project binding<select value={binding} onChange={e=>setBinding(e.target.value)}><option value="">No project binding</option>{projects.map(p=><option key={p.alias} value={p.alias}>{p.name}</option>)}</select></label>}
    {taskKind==="review_page"&&<label>Review destination<select value={page} onChange={e=>setPage(e.target.value as typeof page)}>{automationPages.map(p=><option key={p} value={p.toLowerCase()}>{p}</option>)}</select></label>}
    <p>No credentials or secrets. A task is a reminder only. Recurrence uses a fixed offset, without named timezone or DST adjustment.</p><button type="submit" disabled={!title.trim()||!text.trim()}>Prepare preview</button></fieldset>{error&&<p role="alert">{error}</p>}</form>;
}
