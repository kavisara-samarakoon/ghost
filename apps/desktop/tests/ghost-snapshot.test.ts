import assert from "node:assert/strict";
import { afterEach, test } from "node:test";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { loadGhostSnapshot, selectProject, type GhostSnapshot } from "../src/ghost-snapshot.ts";

// Tauri's official IPC mock uses window. No webview or real storage is accessed.
Object.assign(globalThis, { window: globalThis, isTauri: false });
afterEach(() => {
  clearMocks();
  Object.assign(globalThis, { isTauri: false });
});

const live: GhostSnapshot = {
  mode: "live-local",
  ghost_home: "/fixture/.ghost",
  storage_detected: true,
  project_count: 1,
  projects: [{
    alias: "real", name: "Real project", path: "/fixture/project",
    path_exists: true, workspace_exists: true, status_preview: "Ready for review",
    active_session_goal: null, recent_output_count: 2,
  }],
  warnings: [],
  safety: {
    read_only: true, no_shell_execution: true, no_cli_execution: true,
    no_ai_calls: true, no_network_calls: true, no_file_writes: true,
  },
};

test("browser preview makes no IPC call", async () => {
  let calls = 0;
  mockIPC(() => { calls += 1; throw new Error("Browser should not invoke"); });
  assert.deepEqual(await loadGhostSnapshot(), { snapshot: null, notice: null });
  assert.equal(calls, 0);
});

test("desktop requests only the snapshot command with no paths or arguments", async () => {
  Object.assign(globalThis, { isTauri: true });
  const calls: string[] = [];
  mockIPC((command, args) => {
    calls.push(command);
    assert.deepEqual(args, {});
    return live;
  });
  const state = await loadGhostSnapshot();
  assert.deepEqual(state.snapshot, live);
  assert.deepEqual(calls, ["load_ghost_snapshot"]);
});

test("absent storage keeps samples and gives a calm notice", async () => {
  Object.assign(globalThis, { isTauri: true });
  mockIPC(() => ({ ...live, mode: "static-preview", storage_detected: false, projects: [], project_count: 0 }));
  const state = await loadGhostSnapshot();
  assert.equal(state.snapshot, null);
  assert.match(state.notice!, /No GHOST storage found/);
});

test("malformed storage and rejected IPC fall back without exposing raw details", async () => {
  Object.assign(globalThis, { isTauri: true });
  mockIPC(() => ({ ...live, mode: "static-preview", warnings: ["private-error-marker"] }));
  let state = await loadGhostSnapshot();
  assert.equal(state.snapshot, null);
  assert.match(state.notice!, /Local metadata is unavailable/);
  mockIPC(() => { throw new Error("private-error-marker"); });
  state = await loadGhostSnapshot();
  assert.equal(state.snapshot, null);
  assert.match(state.notice!, /could not be loaded/);
  assert.ok(!state.notice!.includes("private-error-marker"));
});

test("a valid empty registry stays live instead of displaying fictional projects", async () => {
  Object.assign(globalThis, { isTauri: true });
  mockIPC(() => ({ ...live, projects: [], project_count: 0 }));
  const state = await loadGhostSnapshot();
  assert.equal(state.snapshot?.mode, "live-local");
  assert.equal(state.snapshot?.project_count, 0);
  assert.equal(selectProject(state.snapshot!.projects, null), undefined);
});

test("project selection respects user choice and prefers available real metadata", () => {
  const unavailable = { ...live.projects[0], alias: "missing", workspace_exists: false };
  const active = { ...live.projects[0], alias: "active", active_session_goal: "Review release" };
  const projects = [unavailable, live.projects[0], active];
  assert.equal(selectProject(projects, null)?.alias, "active");
  assert.equal(selectProject(projects, "real")?.alias, "real");
  assert.equal(selectProject(projects, "missing")?.alias, "missing");
  assert.equal(selectProject(projects.slice(0, 2), null)?.alias, "real");
});
