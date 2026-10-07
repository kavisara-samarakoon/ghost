import { useRef, useState, type RefObject } from "react";
import type { GhostProject, GhostSnapshot } from "./ghost-snapshot.ts";
import MemorySearch from "./MemorySearch.tsx";

import PersonalMemory from "./PersonalMemory.tsx";
import UnifiedContext from "./UnifiedContext.tsx";
import { PersonalMemoryClient } from "./personal-memory.ts";
import "./personal-memory.css";

type MemoryPageProps = {
  projects: GhostProject[];
  project?: GhostProject;
  mode: GhostSnapshot["mode"];
  searchInputRef: RefObject<HTMLInputElement | null>;
  onNavigate?: (page: "Projects" | "Sessions" | "Artifacts") => void;
};

export default function MemoryPage(props: MemoryPageProps) {
  const [view, setView] = useState("Workspace");
  const client = useRef(new PersonalMemoryClient()).current;
  return <>
    <div className="memory-tabs" role="tablist" aria-label="Memory sources">{["Workspace", "Personal", "Context"].map(name => <button key={name} role="tab" aria-selected={view === name} onClick={() => { client.invalidate(); setView(name); }}>{name}</button>)}</div>
    {view === "Workspace" && <MemorySearch {...props} layout="page" />}
    {view === "Personal" && <PersonalMemory client={client} />}
    {view === "Context" && <UnifiedContext client={client} projectAlias={props.project?.alias} />}
  </>;
}
