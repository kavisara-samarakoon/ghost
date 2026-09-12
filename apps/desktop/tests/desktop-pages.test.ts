import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { registerHooks } from "node:module";
import { afterEach, test } from "node:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import ts from "typescript";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { sampleProjects } from "../src/preview-projects.ts";
import type { GhostProject } from "../src/ghost-snapshot.ts";

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
