import type { RefObject } from "react";
import type { GhostProject, GhostSnapshot } from "./ghost-snapshot.ts";
import MemorySearch from "./MemorySearch.tsx";

type MemoryPageProps = {
  projects: GhostProject[];
  project?: GhostProject;
  mode: GhostSnapshot["mode"];
  searchInputRef: RefObject<HTMLInputElement | null>;
  onNavigate?: (page: "Projects" | "Sessions" | "Artifacts") => void;
};

export default function MemoryPage(props: MemoryPageProps) {
  return <MemorySearch {...props} layout="page" />;
}
