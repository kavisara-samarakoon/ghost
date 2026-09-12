import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { registerHooks } from "node:module";
import { afterEach, test } from "node:test";
import { Children, createElement, isValidElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import ts from "typescript";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { sampleProjects } from "../src/preview-projects.ts";
import { selectProject, type GhostProject } from "../src/ghost-snapshot.ts";

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
const { default: App } = await import("../src/App.tsx");
Object.assign(globalThis, { window: globalThis, isTauri: false });
afterEach(() => { clearMocks(); Object.assign(globalThis, { isTauri: false }); });

function render(page: "Projects" | "Sessions" | "Memory" | "Artifacts", projects = sampleProjects, mode: "live-local" | "static-preview" = "static-preview") {
  return renderToStaticMarkup(createElement(DesktopPages, {
    page, projects, project: projects[0], mode, notice: null, warnings: [], onSelect() {}, searchInputRef: { current: null },
  }));
}

test("all preview pages are labelled samples and render without invoking native actions", () => {
  mockIPC(() => assert.fail("Page rendering must never invoke"));
  for (const page of ["Projects", "Sessions", "Memory", "Artifacts"] as const) {
    const html = render(page);
    assert.match(html, /Static preview · Sample data/);
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
