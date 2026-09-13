import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { registerHooks } from "node:module";
import { afterEach, test } from "node:test";
import { Children, createElement, isValidElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import ts from "typescript";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { sampleProjects } from "../src/preview-projects.ts";
import { searchGhostMemory, type SearchResponse } from "../src/ghost-memory.ts";
import { selectProject, type GhostArtifact, type GhostProject } from "../src/ghost-snapshot.ts";

// Reuse the installed compiler to render the real page components without a new test runtime.
registerHooks({
  resolve(specifier, context, nextResolve) {
    if (context.parentURL?.includes("/src/") && specifier.startsWith("./") && !/\.[a-z]+$/.test(specifier)) {
      const extension = existsSync(new URL(`${specifier}.tsx`, context.parentURL)) ? ".tsx" : ".ts";
      return nextResolve(`${specifier}${extension}`, context);
    }
    return nextResolve(specifier, context);
  },
  load(url, context, nextLoad) {
    if (/\.(css|png)$/.test(url)) return { format: "module", shortCircuit: true, source: `export default ${JSON.stringify(url)};` };
    if (url.endsWith(".tsx")) return { format: "module", shortCircuit: true, source: ts.transpileModule(readFileSync(new URL(url), "utf8"), {
      compilerOptions: { jsx: ts.JsxEmit.ReactJSX, module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2020 },
    }).outputText };
    return nextLoad(url, context);
  },
});
const { default: DesktopPages } = await import("../src/DesktopPages.tsx");
const { default: ProjectsPage, filterProjects, projectOverview } = await import("../src/ProjectsPage.tsx");
const { default: SessionsPage, activeSessionCount, latestArtifactTime } = await import("../src/SessionsPage.tsx");
const { ArtifactsView, filterArtifacts, latestDatedArtifact, selectedArtifact } = await import("../src/ArtifactsPage.tsx");
const { MemorySearchForm, MemorySearchView, currentSearchResponse, memoryQueryExamples } = await import("../src/MemorySearch.tsx");
const { default: App, CommandPage } = await import("../src/App.tsx");
const { default: ActionRequests, RequestReview } = await import("../src/ActionRequests.tsx");
Object.assign(globalThis, { window: globalThis, isTauri: false });
afterEach(() => { clearMocks(); Object.assign(globalThis, { isTauri: false }); });

test("Action Requests renders request-only preparation, pending metadata and an inert preview", () => {
  Object.assign(globalThis, { isTauri: true });
  mockIPC(() => assert.fail("Rendering must never prepare or save a request"));
  const html = renderToStaticMarkup(createElement(ActionRequests, {
    available: true, projects: sampleProjects, project: sampleProjects[0],
    recent: [{ id: "123-1", created_at: "2026-09-13T12:00:00Z", action_type: "create_handoff", project_alias: "example", status: "pending" }],
  }));
  assert.match(html, /Request-only desktop actions/);
  assert.match(html, /Prepare Action/);
  assert.match(html, /Project alias/);
  assert.match(html, /Goal text/);
  assert.match(html, /example · pending/);
  assert.ok(!html.includes("Save Request"));
  const preview = renderToStaticMarkup(createElement(ActionRequests, { available: false, projects: [] }));
  assert.match(preview, /fieldset disabled/);
  assert.match(preview, /Sample projects cannot save/);
  const request = { id: "123-1", created_at: "2026-09-13T12:00:00Z", action_type: "start_session" as const,
    payload: { goal: "<script>unsafe()</script>" }, project_alias: "example", status: "pending" as const,
    preview_title: "Start session request", preview_body: "<script>unsafe()</script>", safety_notice: "Request only." };
  let saves = 0; let edits = 0;
  const props = { request, busy: false, onSave() { saves += 1; }, onEdit() { edits += 1; } };
  const review = renderToStaticMarkup(createElement(RequestReview, props));
  assert.match(review, /Review Action Request/);
  assert.match(review, /Save Request/);
  assert.ok(!review.includes("<script>"));
  assert.match(review, /&lt;script&gt;/);
  assert.equal(saves, 0);
  const buttons = buttonsIn(RequestReview(props));
  buttons[1].onClick(); assert.equal(edits, 1); assert.equal(saves, 0);
  buttons[0].onClick(); assert.equal(saves, 1);
  const busy = renderToStaticMarkup(createElement(RequestReview, { ...props, busy: true }));
  assert.equal((busy.match(/disabled=""/g) ?? []).length, 2);
});

function render(page: "Projects" | "Sessions" | "Memory" | "Artifacts", projects = sampleProjects, mode: "live-local" | "static-preview" = "static-preview") {
  return renderToStaticMarkup(createElement(DesktopPages, {
    page, projects, project: projects[0], mode, notice: null, warnings: [], onSelect() {}, searchInputRef: { current: null },
  }));
}

test("all preview pages are labelled samples and render without invoking native actions", () => {
  mockIPC(() => assert.fail("Page rendering must never invoke"));
  for (const page of ["Projects", "Sessions", "Memory", "Artifacts"] as const) {
    const html = render(page);
    assert.match(html, /Desktop preview|Static preview · Sample data/);
    assert.ok(!html.includes('aria-label="Open '));
    assert.ok(!html.includes('aria-label="Reveal '));
  }
});

test("an empty live registry remains empty across project, session, and artifact pages", () => {
  Object.assign(globalThis, { isTauri: true });
  mockIPC(() => assert.fail("Empty views must not invoke"));
  assert.match(render("Projects", [], "live-local"), /No registered projects/);
  assert.match(render("Sessions", [], "live-local"), /No project selected/);
  assert.match(render("Artifacts", [], "live-local"), /No recent artifacts recorded/);
  for (const page of ["Projects", "Sessions", "Artifacts"] as const) assert.ok(!render(page, [], "live-local").includes("NEXORA"));
});

test("live sessions distinguish missing metadata from no active session", () => {
  const project = { ...sampleProjects[0], active_session: null, counts: { ...sampleProjects[0].counts, sessions: null } };
  assert.match(render("Sessions", [project], "live-local"), /Active session unavailable/);
  assert.match(render("Sessions", [{ ...project, counts: { ...project.counts, sessions: 0 } }], "live-local"), /No active session/);
});

test("artifact pages retain the existing native action guards", () => {
  Object.assign(globalThis, { isTauri: true });
  mockIPC(() => assert.fail("Rendering must not open or reveal"));
  const project: GhostProject = { ...sampleProjects[0], path_exists: true, workspace_exists: true };
  assert.match(render("Artifacts", [project], "live-local"), /aria-label="Open Wishlist alert context"/);
  assert.ok(!render("Artifacts", [{ ...project, workspace_exists: false }], "live-local").includes('aria-label="Open '));
  assert.ok(!render("Artifacts", [project]).includes('aria-label="Open '));
});

test("project names, paths, and notes are rendered as inert text", () => {
  const project = { ...sampleProjects[0], name: '<img src="invalid" onerror="alert(1)">', path: '<script>unsafe</script>' };
  const html = render("Projects", [project], "live-local");
  assert.match(html, /&lt;img/);
  assert.match(html, /&lt;script/);
  assert.ok(!html.includes("<img"));
  assert.ok(!html.includes("<script"));
});

const memoryResponse: SearchResponse = {
  query: "release notes", mode: "live-local", results: [], warnings: [],
  safety: { read_only: true, no_shell_execution: true, no_cli_execution: true,
    no_ai_calls: true, no_network_calls: true, no_file_writes: true },
};

function memoryViewProps(overrides: Partial<Parameters<typeof MemorySearchView>[0]> = {}): Parameters<typeof MemorySearchView>[0] {
  return { mode: "static-preview", project: sampleProjects[0], projects: sampleProjects, layout: "page",
    query: "", scope: "all", pending: false, response: null, inputRef: { current: null },
    onQueryChange() {}, onScopeChange() {}, onSubmit() {}, onDismiss() {}, ...overrides };
}

test("Memory Page v1 renders its overview, search workspace, guidance, and safe preview state", () => {
  mockIPC(() => assert.fail("Visiting Memory must not search or perform native actions"));
  for (const mode of ["static-preview", "live-local"] as const) {
    const html = render("Memory", sampleProjects, mode);
    assert.match(html, /<h1[^>]*>Memory<\/h1>/);
    assert.match(html, /aria-label="Search GHOST memory"/);
    assert.match(html, /aria-label="Memory overview"|aria-label="Sample memory overview"/);
    assert.match(html, /Search local memory/);
    assert.match(html, /Query examples/);
    assert.match(html, /Safe interpretation/);
    assert.match(html, mode === "live-local" ? /Live local read-only/ : /Desktop preview/);
  }
});

test("Memory query helpers pre-fill only and never invoke search", () => {
  Object.assign(globalThis, { isTauri: true });
  mockIPC(() => assert.fail("Query helpers must not invoke IPC"));
  const queries: string[] = [];
  let submits = 0;
  const tree = MemorySearchView(memoryViewProps({ mode: "live-local", onQueryChange: (query) => queries.push(query), onSubmit: () => { submits += 1; } }));
  for (const button of buttonsIn(tree).slice(0, memoryQueryExamples.length)) button.onClick();
  assert.deepEqual(queries, [...memoryQueryExamples]);
  assert.equal(submits, 0);
});

test("Memory form submission reaches only the existing approved search behavior", async () => {
  Object.assign(globalThis, { isTauri: true });
  const calls: unknown[] = [];
  mockIPC((command, args) => { calls.push({ command, args }); return memoryResponse; });
  let prevented = false;
  let search: Promise<SearchResponse> | undefined;
  const form = MemorySearchForm(memoryViewProps({ mode: "live-local", query: "release notes",
    onSubmit: () => { search = searchGhostMemory("live-local", "release notes"); } }));
  form.props.onSubmit({ preventDefault: () => { prevented = true; } } as never);
  assert.equal(prevented, true);
  assert.deepEqual(await search, memoryResponse);
  assert.deepEqual(calls, [{ command: "search_ghost_memory", args: { query: "release notes" } }]);
});

test("Memory stale-response guard accepts only the current generation", () => {
  assert.equal(currentSearchResponse(8, 9, memoryResponse), null);
  assert.equal(currentSearchResponse(9, 9, memoryResponse), memoryResponse);
});


function renderProjects(query = "", alias = sampleProjects[0].alias, projects = sampleProjects) {
  return renderToStaticMarkup(createElement(ProjectsPage, {
    projects, project: selectProject(projects, alias), mode: "static-preview", query,
    onQueryChange() {}, onSelect() {},
  }));
}

test("Projects renders the sample cards, overview, and selected styling", () => {
  const html = renderProjects();
  assert.equal((html.match(/class="glass-panel project-page-card/g) ?? []).length, sampleProjects.length);
  assert.equal((html.match(/aria-pressed="true"/g) ?? []).length, 1);
  assert.match(html, /aria-label="Sample project overview"/);
  assert.match(html, /id="project-filter"/);
  assert.match(html, /ghost project add/);
});

test("project filtering searches name, alias, configured path, and recorded or derived status", () => {
  const projects = [{ ...sampleProjects[0], path: "/work/commerce", path_exists: true, workspace_exists: true, status_preview: "Ready for review" },
    { ...sampleProjects[1], path: "/work/security", path_exists: true, workspace_exists: false }];
  for (const query of ["NEXORA", "nexora", "commerce", "ready review", "active session", "  NeXoRa   review  "]) {
    assert.deepEqual(filterProjects(projects, query, "live-local").map((p) => p.alias), [projects[0].alias]);
  }
  assert.deepEqual(filterProjects(projects, "workspace unavailable", "live-local").map((p) => p.alias), [projects[1].alias]);
  assert.equal(filterProjects(projects, " ", "live-local").length, 2);
  assert.equal(filterProjects(projects, "no-such-project", "live-local").length, 0);
});

test("filtered cards and the no-match state are rendered while preserving selection", () => {
  const html = renderProjects("Sentinel");
  assert.equal((html.match(/class="glass-panel project-page-card/g) ?? []).length, 1);
  assert.match(html, /1 of 5 projects/);
  assert.match(html, /Selection is preserved/);
  assert.match(html, /<strong>NEXORA<\/strong>/);
  const empty = renderProjects("no-such-project");
  assert.match(empty, /No projects match/);
  assert.match(empty, /Clear filter/);
  assert.ok(!empty.includes('class="glass-panel project-page-card'));
});

test("selected project styling follows the existing shared selection helper", () => {
  const html = renderProjects("", sampleProjects[1].alias);
  const cards = html.split('<button type="button" class="glass-panel project-page-card');
  assert.ok(!cards[1].startsWith(' selected'));
  assert.ok(cards[2].startsWith(' selected'));
  assert.match(cards[2], /aria-pressed="true"/);
});

test("overview leaves unknown session totals unavailable and counts only loaded artifact previews", () => {
  assert.deepEqual(projectOverview(sampleProjects), { total: 5, activeSessions: 1, loadedArtifacts: 2 });
  const unknown = { ...sampleProjects[1], counts: { ...sampleProjects[1].counts, sessions: null } };
  assert.equal(projectOverview([sampleProjects[0], unknown]).activeSessions, null);
  assert.equal(projectOverview([unknown]).loadedArtifacts, 0);
  assert.deepEqual(projectOverview([]), { total: 0, activeSessions: 0, loadedArtifacts: 0 });
});

test("visiting and filtering Projects neither mounts memory search nor invokes IPC", () => {
  Object.assign(globalThis, { isTauri: true });
  mockIPC(() => assert.fail("Projects must use only already-loaded frontend metadata"));
  const page = render("Projects", sampleProjects, "live-local");
  assert.ok(!page.includes('aria-label="Search GHOST memory"'));
  for (const query of ["", "nexora", "missing"]) {
    const html = renderProjects(query);
    assert.ok(!html.includes('role="search"'));
    assert.ok(!html.includes('aria-label="Open '));
  }
});


test("Sessions renders the selected active session, notes, status, and related project work", () => {
  for (const mode of ["static-preview", "live-local"] as const) {
    const project = { ...sampleProjects[0], path_exists: true, workspace_exists: true };
    const html = render("Sessions", [project], mode);
    assert.match(html, /<h1[^>]*>Sessions<\/h1>/);
    assert.ok(html.includes(project.active_session!.goal_preview));
    assert.ok(html.includes(project.active_session!.note_preview!));
    assert.ok(html.includes(project.status_preview!));
    assert.ok(html.includes(project.alias));
    assert.ok(html.includes(project.recent_artifacts[0].title));
    assert.match(html, /Related work/);
    assert.match(html, /Date unavailable/);
    assert.match(html, mode === "live-local" ? /Live local read-only/ : /Desktop preview/);
  }
});

test("Sessions keeps known empty, unreadable, and missing project states distinct", () => {
  const empty = { ...sampleProjects[1], active_session_goal: "Obsolete goal must stay hidden" };
  const html = render("Sessions", [empty], "live-local");
  assert.match(html, /No active session/);
  assert.match(html, /<code>ghost session start/);
  assert.ok(!html.includes(empty.active_session_goal));
  for (const sessions of [null, 1]) {
    const unavailable = render("Sessions", [{ ...empty, counts: { ...empty.counts, sessions } }], "live-local");
    assert.match(unavailable, /Active session unavailable/);
    assert.ok(!unavailable.includes("ghost session start"));
  }
  assert.match(render("Sessions", [], "live-local"), /No project selected/);
  const missingNotes = { ...sampleProjects[0], status_preview: null, active_session: { ...sampleProjects[0].active_session!, note_preview: null } };
  const missing = render("Sessions", [missingNotes], "live-local");
  assert.match(missing, /No session notes recorded/);
  assert.match(missing, /No recorded project status/);
});

test("Sessions counts active metadata without treating unavailable values as zero", () => {
  assert.equal(activeSessionCount(sampleProjects), 1);
  assert.equal(activeSessionCount([]), 0);
  const unreadable = { ...sampleProjects[1], counts: { ...sampleProjects[1].counts, sessions: null } };
  assert.equal(activeSessionCount([sampleProjects[0], unreadable]), null);
  assert.equal(activeSessionCount([{ ...unreadable, active_session: sampleProjects[0].active_session }]), 1);
  assert.equal(activeSessionCount([{ ...unreadable, counts: { ...unreadable.counts, sessions: 1 } }]), 1);
});

test("Sessions finds the latest valid artifact timestamp without changing artifact order", () => {
  const artifact = sampleProjects[0].recent_artifacts[0];
  const dates = ["2026-09-10T10:00:00Z", null, "invalid", "2026-09-12T10:00:00Z", "2026-09-11T10:00:00Z"];
  const artifacts = dates.map((created_at) => ({ ...artifact, created_at }));
  assert.equal(latestArtifactTime(artifacts), dates[3]);
  assert.deepEqual(artifacts.map((item) => item.created_at), dates);
  assert.equal(latestArtifactTime([]), null);
  assert.equal(latestArtifactTime([{ ...artifact, created_at: "invalid" }]), null);
});

test("Sessions uses the existing Open and Reveal guards without invoking them on render", () => {
  Object.assign(globalThis, { isTauri: true });
  mockIPC(() => assert.fail("Rendering session artifacts must never invoke"));
  const project = { ...sampleProjects[0], path_exists: true, workspace_exists: true };
  for (const action of ["Open", "Reveal"]) {
    assert.ok(render("Sessions", [project], "live-local").includes(`aria-label="${action} ${project.recent_artifacts[0].title}"`));
    assert.ok(!render("Sessions", [project]).includes(`aria-label="${action} `));
    assert.ok(!render("Sessions", [{ ...project, workspace_exists: false }], "live-local").includes(`aria-label="${action} `));
  }
});

// Exercise actual navigation button callbacks without adding a DOM dependency.
function buttonsIn(node: ReactNode): Array<{ onClick: () => void }> {
  const buttons: Array<{ onClick: () => void }> = [];
  Children.forEach(node, (child) => {
    if (!isValidElement<{ children?: ReactNode; onClick?: () => void }>(child)) return;
    if (child.type === "button" && child.props.onClick) buttons.push({ onClick: child.props.onClick });
    buttons.push(...buttonsIn(child.props.children));
  });
  return buttons;
}

function buttonsWithClass(node: ReactNode, className: string): Array<{ onClick: () => void }> {
  const buttons: Array<{ onClick: () => void }> = [];
  Children.forEach(node, (child) => {
    if (!isValidElement<{ children?: ReactNode; className?: string; onClick?: () => void }>(child)) return;
    if (child.type === "button" && child.props.className === className && child.props.onClick) {
      buttons.push({ onClick: child.props.onClick });
    }
    buttons.push(...buttonsWithClass(child.props.children, className));
  });
  return buttons;
}

function commandView(project = sampleProjects[0], onNavigate: (page: "Projects" | "Sessions" | "Memory" | "Artifacts") => void = () => {}) {
  return CommandPage({ projects: sampleProjects, project, mode: "static-preview", notice: null, warnings: [], onSelect() {}, onNavigate });
}

test("Command renders the local workflow, MVP guidance, and real page destinations", () => {
  mockIPC(() => assert.fail("Visiting Command must not search memory or invoke native actions"));
  const html = renderToStaticMarkup(commandView());
  assert.match(html, /<h1[^>]*id="command-title"[^>]*>.*Command Space/s);
  assert.match(html, /Current workflow/);
  assert.match(html, /Suggested next steps/);
  assert.match(html, /Local MVP status/);
  assert.match(html, /Recommended flow/);
  for (const label of ["Review Projects", "Continue Session", "Search Memory", "Review Artifacts"]) {
    assert.ok(html.includes(`aria-label="${label}"`));
  }
  assert.ok(!html.includes('role="search"'));
});

test("Command uses loaded snapshot values and calm placeholders without inventing records", () => {
  const liveProject = { ...sampleProjects[0], path: "/work/ghost", path_exists: true, workspace_exists: true };
  const live = renderToStaticMarkup(CommandPage({ projects: [liveProject], project: liveProject, mode: "live-local",
    notice: null, warnings: [], onSelect() {}, onNavigate() {} }));
  assert.ok(live.includes(liveProject.name));
  assert.ok(live.includes(liveProject.path));
  assert.ok(live.includes(liveProject.active_session!.id));
  assert.ok(live.includes(liveProject.recent_artifacts[0].title));
  assert.match(live, /Live local read-only/);

  const empty = renderToStaticMarkup(CommandPage({ projects: [], project: undefined, mode: "live-local",
    notice: null, warnings: [], onSelect() {}, onNavigate() {} }));
  assert.match(empty, /No active session/);
  assert.match(empty, /No artifact selected/);
  assert.ok(!empty.includes("NEXORA"));
});

test("Command navigation cards switch frontend pages while preserving project selection and avoiding IPC", () => {
  Object.assign(globalThis, { isTauri: true });
  mockIPC(() => assert.fail("Command navigation must not invoke native commands or memory search"));
  const project = sampleProjects[1];
  const destinations: string[] = [];
  const tree = commandView(project, (page) => {
    destinations.push(page);
    const selected = selectProject(sampleProjects, project.alias);
    assert.equal(selected, project);
    assert.match(renderToStaticMarkup(createElement(DesktopPages, {
      page, projects: sampleProjects, project: selected, mode: "live-local", notice: null, warnings: [],
      onSelect() {}, searchInputRef: { current: null },
    })), new RegExp(`<h1[^>]*>${page}</h1>`));
  });
  for (const button of buttonsWithClass(tree, "command-action-card")) button.onClick();
  assert.deepEqual(destinations, ["Projects", "Sessions", "Memory", "Artifacts"]);
});

test("Memory navigation keeps the selected project and performs no IPC", () => {
  Object.assign(globalThis, { isTauri: true });
  mockIPC(() => assert.fail("Memory navigation must not invoke native commands"));
  const project = sampleProjects[1];
  const destinations: string[] = [];
  const tree = MemorySearchView(memoryViewProps({ mode: "live-local", project, onNavigate(page) {
    destinations.push(page);
    const selected = selectProject(sampleProjects, project.alias);
    assert.equal(selected, project);
    assert.match(render(page, sampleProjects, "live-local"), new RegExp(`<h1[^>]*>${page}</h1>`));
  } }));
  for (const button of buttonsIn(tree).slice(memoryQueryExamples.length)) button.onClick();
  assert.deepEqual(destinations, ["Projects", "Sessions", "Artifacts"]);
});

test("Command, Projects, Sessions, Memory, and Artifacts all render after cockpit polish", () => {
  mockIPC(() => assert.fail("Rendering existing pages must not invoke native commands"));
  assert.match(renderToStaticMarkup(createElement(App)), /Command Space/);
  for (const page of ["Projects", "Sessions", "Memory", "Artifacts"] as const) {
    assert.match(render(page), new RegExp(`<h1[^>]*>${page}</h1>`));
  }
});

test("Sessions navigation requests existing pages while retaining selection and avoiding IPC", () => {
  Object.assign(globalThis, { isTauri: true });
  mockIPC(() => assert.fail("Session navigation must never invoke or search memory"));
  const destinations: string[] = [];
  const project = sampleProjects[1];
  const tree = SessionsPage({ projects: sampleProjects, project, mode: "live-local", onNavigate(page) {
    destinations.push(page);
    const selected = selectProject(sampleProjects, project.alias);
    assert.equal(selected, project);
    const html = renderToStaticMarkup(createElement(DesktopPages, {
      page, projects: sampleProjects, project: selected, mode: "live-local", notice: null, warnings: [],
      onSelect() {}, searchInputRef: { current: null },
    }));
    assert.match(html, new RegExp(`<h1[^>]*>${page}</h1>`));
  } });
  for (const button of buttonsIn(tree)) button.onClick();
  assert.deepEqual(destinations, ["Artifacts", "Memory", "Projects"]);
});

test("Sessions does not mount memory search and Command and Projects still render", () => {
  Object.assign(globalThis, { isTauri: true });
  mockIPC(() => assert.fail("Rendering pages must not invoke or search memory"));
  assert.ok(!render("Sessions", sampleProjects, "live-local").includes('role="search"'));
  assert.match(render("Projects"), /id="project-filter"/);
  assert.match(renderToStaticMarkup(createElement(App)), /Command Space/);
});

test("Sessions renders notes, goals, status and project notices as inert text", () => {
  const text = '<img src="invalid" onerror="alert(1)">';
  const project = { ...sampleProjects[0], status_preview: text, warnings: [text],
    active_session: { ...sampleProjects[0].active_session!, goal_preview: text, note_preview: text } };
  const html = render("Sessions", [project], "live-local");
  assert.ok(!html.includes("<img"));
  assert.ok((html.match(/&lt;img/g) ?? []).length >= 4);
});


const artifactFixtures: GhostArtifact[] = [
  { kind: "context-pack", title: "Release context", relative_path: "drafts/context-packs/release.md", created_at: "2026-09-10T10:00:00Z", preview: "Recorded release scope" },
  { kind: "handoff", title: "Review handoff", relative_path: "drafts/handoffs/codex/review.md", created_at: null, preview: "Review checklist" },
  { kind: "output", title: "Validation output", relative_path: "outputs/codex/20260912T100000000000Z-codex-output-abcdefgh.md", created_at: "2026-09-12T10:00:00Z", preview: "Recorded validation" },
  { kind: "next-step", title: "Next review", relative_path: "drafts/next-steps/review.md", created_at: "invalid", preview: null },
  { kind: "update-pack", title: "Milestone update", relative_path: "drafts/update-packs/m23/summary.md", created_at: "2026-09-11T10:00:00Z", preview: "Milestone summary" },
];
const artifactProject: GhostProject = { ...sampleProjects[0], name: "Release Workspace", alias: "release", path_exists: true, workspace_exists: true, recent_artifacts: artifactFixtures };

function artifactViewProps(overrides: Partial<Parameters<typeof ArtifactsView>[0]> = {}): Parameters<typeof ArtifactsView>[0] {
  return { project: artifactProject, mode: "static-preview", query: "", category: "all", selectedPath: null,
    onQueryChange() {}, onCategoryChange() {}, onSelect() {}, onAction() {}, pendingPath: null, result: null, ...overrides };
}

function renderArtifacts(overrides: Partial<Parameters<typeof ArtifactsView>[0]> = {}) {
  return renderToStaticMarkup(createElement(ArtifactsView, artifactViewProps(overrides)));
}

test("Artifacts renders loaded work and a selected detail in preview and live modes", () => {
  for (const mode of ["static-preview", "live-local"] as const) {
    const html = render("Artifacts", [artifactProject], mode);
    assert.match(html, /<h1[^>]*>Artifacts<\/h1>/);
    assert.match(html, /Generated work/);
    assert.equal((html.match(/class="glass-panel artifact-select-card/g) ?? []).length, 5);
    for (const artifact of artifactFixtures) assert.ok(html.includes(artifact.title));
    assert.match(html, /id="selected-artifact"/);
    assert.match(html, /Recorded release scope/);
    assert.match(html, /Date unavailable/);
    assert.ok(!html.includes('role="search"'));
  }
});

test("artifact search combines title, type, project and safe path terms with category filtering", () => {
  for (const [query, expected] of [["validation", 1], ["HANDOFF", 1], ["Workspace", 5], ["release", 5], ["m23/summary", 1], ["  ReLeAsE   context  ", 1], ["missing", 0]] as const) {
    assert.equal(filterArtifacts(artifactProject, query, "all").length, expected);
  }
  for (const artifact of artifactFixtures) assert.deepEqual(filterArtifacts(artifactProject, "", artifact.kind), [artifact]);
  assert.deepEqual(filterArtifacts(artifactProject, "milestone", "output"), []);
  assert.deepEqual(filterArtifacts(undefined, "", "all"), []);
});

test("artifact selection stays visible or falls back to the first match and clears on no matches", () => {
  const selected = artifactFixtures[4];
  assert.equal(selectedArtifact(artifactFixtures, selected.relative_path), selected);
  const html = renderArtifacts({ selectedPath: selected.relative_path });
  assert.match(html, /id="selected-artifact-title">Milestone update/);
  assert.match(html, /Milestone summary/);
  const filtered = filterArtifacts(artifactProject, "validation", "all");
  assert.equal(selectedArtifact(filtered, selected.relative_path), artifactFixtures[2]);
  assert.equal(selectedArtifact([], selected.relative_path), undefined);
  const empty = renderArtifacts({ query: "no-match" });
  assert.match(empty, /No artifacts match/);
  assert.ok(!empty.includes('id="selected-artifact"'));
  assert.ok(!empty.includes('class="glass-panel artifact-select-card'));
  assert.match(renderArtifacts({ project: sampleProjects[1] }), /No recent artifacts recorded/);
});

test("artifact filter, selection, and clear controls call only their frontend callbacks", () => {
  const categories: string[] = [];
  const selected: string[] = [];
  const queries: string[] = [];
  Object.assign(globalThis, { isTauri: true });
  mockIPC(() => assert.fail("Filtering and selecting must not invoke"));
  const tree = ArtifactsView(artifactViewProps({ onCategoryChange: (value) => categories.push(value), onSelect: (value) => selected.push(value) }));
  for (const button of buttonsIn(tree)) button.onClick();
  assert.deepEqual(categories, ["all", "context-pack", "handoff", "output", "next-step", "update-pack"]);
  assert.deepEqual(selected, artifactFixtures.map((artifact) => artifact.relative_path));
  categories.length = 0;
  const empty = ArtifactsView(artifactViewProps({ query: "no-match", category: "output", onQueryChange: (value) => queries.push(value), onCategoryChange: (value) => categories.push(value) }));
  buttonsIn(empty)[0].onClick();
  assert.deepEqual(queries, [""]);
  assert.deepEqual(categories, ["all"]);
});

test("latest artifact uses only valid timestamps and leaves snapshot order unchanged", () => {
  const original = [...artifactFixtures];
  assert.equal(latestDatedArtifact(artifactFixtures), artifactFixtures[2]);
  assert.deepEqual(artifactFixtures, original);
  assert.equal(latestDatedArtifact([artifactFixtures[1], artifactFixtures[3]]), undefined);
  assert.equal(latestDatedArtifact([]), undefined);
  assert.match(renderArtifacts({ project: sampleProjects[0] }), /No dated artifacts loaded/);
});

test("Artifacts keeps action guards, pending state and feedback tied to the selected path", () => {
  Object.assign(globalThis, { isTauri: true });
  mockIPC(() => assert.fail("Rendering must not perform artifact actions"));
  for (const artifact of artifactFixtures) {
    const html = renderArtifacts({ mode: "live-local", selectedPath: artifact.relative_path });
    assert.ok(html.includes(`aria-label="Open ${artifact.title}"`));
    assert.ok(html.includes(`aria-label="Reveal ${artifact.title}"`));
  }
  assert.ok(!renderArtifacts().includes('aria-label="Open '));
  for (const project of [{ ...artifactProject, workspace_exists: false }, { ...artifactProject, path_exists: false }, { ...artifactProject, alias: "Invalid Alias" }]) {
    assert.ok(!renderArtifacts({ mode: "live-local", project }).includes('aria-label="Open '));
  }
  const pending = renderArtifacts({ mode: "live-local", pendingPath: artifactFixtures[0].relative_path });
  assert.equal((pending.match(/disabled=""/g) ?? []).length, 2);
  assert.match(pending, /Checking artifact/);
  const result = { path: artifactFixtures[0].relative_path, state: "rejected" as const };
  assert.match(renderArtifacts({ mode: "live-local", result }), /Action rejected/);
  assert.ok(!renderArtifacts({ mode: "live-local", result, selectedPath: artifactFixtures[1].relative_path }).includes("Action rejected"));
});

test("Artifacts hides unsafe paths from rendering and filtering and renders previews as inert text", () => {
  const unsafe = ["/private/unsafe.md", "drafts/context-packs/../private.md"];
  for (const relative_path of unsafe) {
    const artifact = { ...artifactFixtures[0], relative_path, preview: '<img src="invalid" onerror="alert(1)">' };
    const project = { ...artifactProject, recent_artifacts: [artifact] };
    const html = renderArtifacts({ mode: "live-local", project });
    assert.ok(!html.includes(relative_path));
    assert.match(html, /Relative path unavailable/);
    assert.ok(!html.includes('aria-label="Open '));
    assert.ok(!html.includes("<img"));
    assert.match(html, /&lt;img/);
    assert.deepEqual(filterArtifacts(project, "private", "all"), []);
  }
});

test("Artifacts navigation preserves project selection without IPC or memory search", () => {
  Object.assign(globalThis, { isTauri: true });
  mockIPC(() => assert.fail("Artifact navigation must never invoke"));
  const destinations: string[] = [];
  const tree = ArtifactsView(artifactViewProps({ onNavigate(page) {
    destinations.push(page);
    const project = selectProject([artifactProject, sampleProjects[1]], artifactProject.alias);
    assert.equal(project, artifactProject);
    assert.match(render(page, [project!]), new RegExp(`<h1[^>]*>${page}</h1>`));
  } }));
  for (const button of buttonsIn(tree)) button.onClick();
  assert.deepEqual(destinations, ["Sessions", "Memory", "Projects"]);
  assert.ok(!render("Artifacts", [artifactProject], "live-local").includes('role="search"'));
});
