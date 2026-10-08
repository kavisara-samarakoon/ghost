import { invoke, isTauri } from "@tauri-apps/api/core";
import type { DesktopPage } from "./DesktopPages.tsx";
import type { GhostProject } from "./ghost-snapshot.ts";
import type { RecentRequest } from "./action-requests.ts";
export const AUTOMATION_POLL_MS = 60_000;
export const automationPages = ["Today", "Mail", "Calendar", "Projects", "Memory", "Sessions", "Artifacts", "Connections", "Command"] as const;
export const weekdays = ["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"] as const;
export type Trigger = {kind:"once";at:string} | {kind:"daily";hour:number;minute:number;offset_minutes:number} | {kind:"weekly";weekday:typeof weekdays[number];hour:number;minute:number;offset_minutes:number} | {kind:"project_no_active_session";project_alias:string} | {kind:"recent_pending_requests_present"};
export type Task = {kind:"reminder";message:string} | {kind:"command_prompt";prompt:string;project_alias:string|null} | {kind:"review_page";page:Lowercase<typeof automationPages[number]>;message:string};
export interface AutomationPayload {title:string;enabled:boolean;trigger:Trigger;task:Task}
export interface AutomationDefinition {version:1;id:string;revision:string;payload:AutomationPayload;created_at:number;updated_at:number;cursor:{last_due:number|null;condition_true:boolean;edge:number}}
export interface AutomationView {definition:AutomationDefinition;next_due:number|null}
export interface AutomationItem {version:1;id:string;automation_id:string;automation_title:string;task:Task;triggered_at:number;occurrence_key:string;status:"pending"|"acknowledged"|"dismissed"}
export type AutomationOperation = "create"|"update"|"pause"|"resume"|"delete";
export interface AutomationPrepare {operation:AutomationOperation;automation_id:string|null;payload:AutomationPayload|null}
export interface AutomationPreview {version:1;request_id:string;operation:AutomationOperation;automation_id:string;before:AutomationDefinition|null;after:AutomationDefinition|null;created_at:number;expires_at:number;request_sha256:string;confirmation_phrase:string}
export interface AutomationOutcome {automation_id:string;changed:boolean;audit_recorded:boolean}
export interface AutomationState {definitions:AutomationView[];inbox:AutomationItem[]}
export interface CommandPrefill {id:string;text:string}
export function automationPhrase(p:AutomationPreview) {return p.operation.toUpperCase()+" AUTOMATION "+p.request_sha256;}
export function offsetLabel(minutes:number) {return "UTC"+(minutes<0?"-":"+")+String(Math.floor(Math.abs(minutes)/60)).padStart(2,"0")+":"+String(Math.abs(minutes)%60).padStart(2,"0");}
export function triggerSummary(t:Trigger):string {
  if(t.kind==="once") return "Once · "+t.at;
  if(t.kind==="project_no_active_session") return "When "+t.project_alias+" has no active session";
  if(t.kind==="recent_pending_requests_present") return "When recent pending local requests are present (global)";
  return (t.kind==="weekly"?t.weekday+" · ":"Daily · ")+String(t.hour).padStart(2,"0")+":"+String(t.minute).padStart(2,"0")+" at "+offsetLabel(t.offset_minutes);
}
export function taskText(t:Task) {return t.kind==="command_prompt"?t.prompt:t.message;}
export function pageForTask(t:Task):DesktopPage|null {
  if(t.kind==="command_prompt") return "Command";
  if(t.kind==="reminder") return null;
  return automationPages.find(page=>page.toLowerCase()===t.page)??null;
}
export function openAutomationItem(item:AutomationItem,onOpen:(page:DesktopPage,prefill:CommandPrefill|null,alias:string|null)=>void) {
  if(item.status!=="pending")return;
  const page=pageForTask(item.task);if(!page)return;
  onOpen(page,item.task.kind==="command_prompt"?{id:item.id,text:item.task.prompt}:null,item.task.kind==="command_prompt"?item.task.project_alias:null);
}
export function needsAttention(project:GhostProject|undefined,recent:RecentRequest[],pending:number):string[] {
  const items:string[]=[];
  if(pending)items.push(String(pending)+" automation attention items are pending.");
  if(recent.length)items.push("Recent pending local workflow requests need review.");
  if(project?.counts.sessions===0)items.push("Selected project has no active session.");
  if(project?.warnings.length)items.push("Selected project has local metadata warnings.");
  return items;
}
export function automationError(value:unknown):string {
  const messages:Record<string,string>={unavailable:"Automations require the native desktop app.",invalid_input:"Check the title, project binding and required fields. Do not include secrets.",invalid_trigger:"Select a supported local trigger.",invalid_task:"Check the task text and supported destination. Do not include secrets.",invalid_schedule:"Check the date, time and fixed UTC offset.",invalid_state:"Local state is unavailable or changed. Review it before preparing again.",changed_review:"The review was consumed or changed. Prepare a fresh preview.",review_expired:"The five-minute review expired. Prepare again.",storage:"Private storage could not be verified. State may have changed; review current items before retrying.",audit:"Audit could not be recorded. No automatic retry occurs.",busy:"Another local operation is running. Try again when it finishes.",limit:"The private document or record limit was reached. Nothing was silently discarded."};
  return typeof value==="string"&&messages[value]?messages[value]:"Automation operation failed. Review current local state; no task was executed.";
}
type Invoker=<T>(command:string,args?:Record<string,unknown>)=>Promise<T>;
function freeze<T>(value:T):T {if(value&&typeof value==="object"){Object.values(value).forEach(freeze);Object.freeze(value);}return value;}
export class AutomationClient {
  private generation=0;private pending:AutomationPreview|null=null;private preparing=false;
  private native:()=>boolean;private transport:Invoker;
  constructor(native:()=>boolean=isTauri,transport:Invoker=invoke){this.native=native;this.transport=transport;}
  available(){return this.native();}
  private ready(){if(!this.native())throw "unavailable";}
  invalidate(){this.generation++;this.pending=null;}
  async status(){this.ready();return this.transport<{definitions:number;pending_count:number;plaintext:boolean;local_only:boolean}>("get_automation_status",{input:{}});}
  async definitions(){this.ready();return this.transport<AutomationView[]>("list_automations",{input:{}});}
  async inbox(){this.ready();return this.transport<AutomationItem[]>("list_automation_inbox",{input:{}});}
  async load():Promise<AutomationState>{const definitions=await this.definitions();const inbox=await this.inbox();return {definitions,inbox};}
  async evaluate(){this.ready();return this.transport<{created:AutomationItem[];pending_count:number;audit_recorded:boolean}>("evaluate_automations",{input:{}});}
  async prepare(input:AutomationPrepare){if(this.preparing)throw "busy";this.ready();this.invalidate();const generation=this.generation;this.preparing=true;
    try{const p=await this.transport<AutomationPreview>("prepare_automation_mutation",{input});if(generation!==this.generation||p.confirmation_phrase!==automationPhrase(p))throw "changed_review";this.pending=freeze(p);return this.pending;}finally{this.preparing=false;}}
  async execute(p:AutomationPreview,confirmation:string){this.ready();if(this.pending!==p||confirmation!==automationPhrase(p))throw "changed_review";this.invalidate();return this.transport<AutomationOutcome>("execute_automation_mutation",{input:{request_id:p.request_id,request_sha256:p.request_sha256,confirmation}});}
  async acknowledge(item_id:string){this.ready();return this.transport<boolean>("acknowledge_automation_item",{input:{item_id}});}
  async dismiss(item_id:string){this.ready();return this.transport<boolean>("dismiss_automation_item",{input:{item_id}});}
}
export interface PollEnvironment {visible:()=>boolean;now:()=>number;interval:(callback:()=>void,ms:number)=>unknown;clear:(id:unknown)=>void;listen:(name:"focus"|"visibilitychange",callback:()=>void)=>()=>void}
export function startAutomationPolling<T>(available:boolean,evaluate:()=>Promise<T>,onResult:(result:T)=>void,onError:(error:unknown)=>void,env:PollEnvironment) {
  if(!available)return ()=>{};
  let stopped=false,inFlight=false,lastAttempt=Number.NEGATIVE_INFINITY;let timer:unknown;
  const poll=async()=>{const now=env.now();if(stopped||inFlight||!env.visible()||now-lastAttempt<AUTOMATION_POLL_MS)return;lastAttempt=now;inFlight=true;
    try{const result=await evaluate();if(!stopped)onResult(result);}catch(error){if(!stopped)onError(error);}finally{inFlight=false;}};
  const visibility=()=>{if(timer!==undefined){env.clear(timer);timer=undefined;}if(env.visible()){void poll();timer=env.interval(()=>void poll(),AUTOMATION_POLL_MS);}};
  const unFocus=env.listen("focus",()=>void poll());const unVisible=env.listen("visibilitychange",visibility);visibility();
  return ()=>{stopped=true;if(timer!==undefined)env.clear(timer);unFocus();unVisible();};
}
