import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { registerHooks } from "node:module";
import { afterEach, test } from "node:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import ts from "typescript";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { actOnSearchResult, canActOnSearchResult, searchGhostMemory, searchValidation, type SearchResponse, type SearchResult } from "../src/ghost-memory.ts";

// Use the project's compiler to render real TSX components in Node, without another test runtime.
registerHooks({ load(url, context, nextLoad) {
  if (url.endsWith(".tsx")) return { format: "module", shortCircuit: true, source: ts.transpileModule(readFileSync(new URL(url), "utf8"), {
    compilerOptions: { jsx: ts.JsxEmit.ReactJSX, module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2020 },
  }).outputText };
  return nextLoad(url, context);
} });
const { default: SearchResults } = await import("../src/SearchResults.tsx");
const { default: MemorySearch } = await import("../src/MemorySearch.tsx");

Object.assign(globalThis, { window: globalThis, isTauri: false });
afterEach(() => { clearMocks(); Object.assign(globalThis, { isTauri: false }); });

const result: SearchResult = { project_alias: "example", project_name: "Example project", kind: "context-pack", title: "Release review",
  relative_path: "drafts/context-packs/review.md", snippet: "Review local evidence. [REDACTED]", created_at: null, openable: true };
const response: SearchResponse = { query: "review", mode: "live-local", results: [result], warnings: [],
  safety: { read_only: true, no_shell_execution: true, no_cli_execution: true, no_ai_calls: true, no_network_calls: true, no_file_writes: true } };

test("browser and static preview never call native search or artifact actions", async () => {
  mockIPC(() => { assert.fail("Preview must not invoke native commands"); });
  for (const desktop of [false, true]) {
    Object.assign(globalThis, { isTauri: desktop });
    const mode = desktop ? "static-preview" : "live-local";
    const found = await searchGhostMemory(mode, "review");
    assert.equal(found.mode, "unavailable");
    assert.match(found.warnings[0], /desktop app/);
    assert.equal(canActOnSearchResult(found, result), false);
    for (const action of ["open", "reveal"] as const) assert.equal(await actOnSearchResult(found, result, action), "unavailable");
  }
  Object.assign(globalThis, { isTauri: false });
  assert.equal(await actOnSearchResult(response, result, "open"), "unavailable");
  assert.equal(canActOnSearchResult(response, result), false);
});

test("desktop searches only through search_ghost_memory with query and optional project_alias", async () => {
  Object.assign(globalThis, { isTauri: true });
  const calls: unknown[] = [];
  mockIPC((command, args) => { calls.push({ command, args }); return response; });
  assert.deepEqual(await searchGhostMemory("live-local", " review "), response);
  assert.deepEqual(await searchGhostMemory("live-local", "review", "example"), response);
  assert.deepEqual(calls, [
    { command: "search_ghost_memory", args: { query: "review" } },
    { command: "search_ghost_memory", args: { query: "review", project_alias: "example" } },
  ]);
});

test("empty and short queries show calm validation without invoking or scanning", async () => {
  Object.assign(globalThis, { isTauri: true });
  mockIPC(() => { assert.fail("Invalid query must not invoke"); });
  for (const query of ["", " ", "a", " é "]) {
    const found = await searchGhostMemory("live-local", query);
    assert.deepEqual(found.results, []);
    assert.equal(found.warnings[0], "Enter at least 2 characters to search local memory.");
    assert.match(renderToStaticMarkup(createElement(SearchResults, { response: found, onDismiss() {} })), /Enter at least 2 characters/);
  }
  for (const query of ["x".repeat(121), "test\nquery"]) assert.ok(searchValidation(query));
  assert.equal((await searchGhostMemory("live-local", "review", "/absolute/path")).mode, "unavailable");
});

test("search errors use generic messages without exposing local details", async () => {
  Object.assign(globalThis, { isTauri: true });
  mockIPC(() => { throw new Error("/private/secret-marker"); });
  const found = await searchGhostMemory("live-local", "review");
  assert.equal(found.mode, "unavailable");
  assert.ok(!JSON.stringify(found).includes("secret-marker"));
});

test("results render titles, snippets, project and relative path as inert text", () => {
  const item = { ...result, snippet: '<img src="https://invalid.example/tracker" onerror="alert(1)"> [REDACTED]' };
  const html = renderToStaticMarkup(createElement(SearchResults, { response: { ...response, results: [item] }, onDismiss() {} }));
  assert.match(html, /Release review/);
  assert.match(html, /Example project/);
  assert.match(html, /drafts\/context-packs\/review.md/);
  assert.match(html, /\[REDACTED\]/);
  assert.match(html, /&lt;img/);
  assert.ok(!html.includes("<img"));
  assert.ok(!html.includes('aria-label="Open'));
});

test("Open and Reveal render only for allowlisted openable live desktop results", () => {
  mockIPC(() => { assert.fail("Rendering must not perform actions"); });
  for (const desktop of [false, true]) for (const live of [false, true]) for (const openable of [false, true]) {
    Object.assign(globalThis, { isTauri: desktop });
    const item = { ...result, openable };
    const found: SearchResponse = { ...response, mode: live ? "live-local" : "unavailable", results: [item] };
    const html = renderToStaticMarkup(createElement(SearchResults, { response: found, onDismiss() {} }));
    assert.equal(html.includes('aria-label="Open Release review"'), desktop && live && openable);
    assert.equal(html.includes('aria-label="Reveal Release review"'), desktop && live && openable);
  }
  Object.assign(globalThis, { isTauri: true });
  for (const path of ["/tmp/review.md", "../.env", "src/App.tsx", "status.md", "drafts/context-packs/.env.md"]) {
    const item = { ...result, relative_path: path };
    assert.equal(canActOnSearchResult({ ...response, results: [item] }, item), false);
  }
});

test("eligible result actions send only M14 commands with alias and relative path", async () => {
  Object.assign(globalThis, { isTauri: true });
  const calls: unknown[] = [];
  mockIPC((command, args) => { calls.push({ command, args }); return command === "open_ghost_artifact" ? "opened" : "revealed"; });
  assert.equal(await actOnSearchResult(response, result, "open"), "opened");
  assert.equal(await actOnSearchResult(response, result, "reveal"), "revealed");
  assert.deepEqual(calls, [
    { command: "open_ghost_artifact", args: { project_alias: "example", relative_path: result.relative_path } },
    { command: "reveal_ghost_artifact", args: { project_alias: "example", relative_path: result.relative_path } },
  ]);
  mockIPC(() => { assert.fail("Unlisted or ineligible selections cannot invoke"); });
  assert.equal(await actOnSearchResult(response, { ...result }, "open"), "rejected");
  const blocked = { ...result, openable: false };
  assert.equal(await actOnSearchResult({ ...response, results: [blocked] }, blocked, "open"), "rejected");
});

test("result action errors and stale paths select safe feedback", async () => {
  Object.assign(globalThis, { isTauri: true });
  mockIPC(() => "rejected");
  assert.equal(await actOnSearchResult(response, result, "open"), "rejected");
  mockIPC(() => { throw new Error("/private/secret-marker"); });
  assert.equal(await actOnSearchResult(response, result, "open"), "unavailable");
});

test("Command Space search renders an enabled form and performs no startup searches", () => {
  mockIPC(() => { assert.fail("Search must require submission"); });
  const html = renderToStaticMarkup(createElement(MemorySearch, { mode: "static-preview" }));
  assert.match(html, /role="search"/);
  assert.match(html, /type="submit"/);
  assert.match(html, /All projects/);
  assert.match(html, /Desktop search preview/);
  assert.ok(!html.includes("disabled"));
});
