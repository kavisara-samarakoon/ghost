import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { registerHooks } from "node:module";
import { afterEach, test } from "node:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import ts from "typescript";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { sampleProjects } from "../src/preview-projects.ts";
import { selectProject, type GhostProject } from "../src/ghost-snapshot.ts";

// Reuse the installed compiler to render the real page components without a new test runtime.
registerHooks({
  resolve(specifier, context, nextResolve) {
    if (context.parentURL?.includes("/src/") && specifier.startsWith("./") && !/\.[a-z]+$/.test(specifier)) {
      return nextResolve(`${specifier}.ts`, context);
    }
    return nextResolve(specifier, context);
  },
  load(url, context, nextLoad) {
    if (url.endsWith(".tsx")) return { format: "module", shortCircuit: true, source: ts.transpileModule(readFileSync(new URL(url), "utf8"), {
      compilerOptions: { jsx: ts.JsxEmit.ReactJSX, module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2020 },
    }).outputText };
    return nextLoad(url, context);
  },
});
const { default: DesktopPages } = await import("../src/DesktopPages.tsx");
const { default: ProjectsPage, filterProjects, projectOverview } = await import("../src/ProjectsPage.tsx");
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
