import assert from "node:assert/strict";
import { afterEach, test } from "node:test";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { actOnArtifact, actionMessages, canActOnArtifact, safeArtifactPath } from "../src/artifact-actions.ts";
import type { GhostArtifact, GhostProject } from "../src/ghost-snapshot.ts";

Object.assign(globalThis, { window: globalThis, isTauri: false });
afterEach(() => { clearMocks(); Object.assign(globalThis, { isTauri: false }); });
const artifact: GhostArtifact = { kind: "context-pack", title: "Review draft",
  relative_path: "drafts/context-packs/review.md", preview: "Ready", created_at: null };
const project: GhostProject = { alias: "example", name: "Example", path: "/never-send-this-path",
  path_exists: true, workspace_exists: true, status_preview: null, active_session_goal: null,
  recent_output_count: 0, active_session: null, recent_artifacts: [artifact], warnings: [],
  counts: { sessions: 0, outputs: 0, handoffs: 0, context_packs: 1, next_steps: 0, update_packs: 0 } };

test("static samples and a browser never request native artifact actions", async () => {
  mockIPC(() => { assert.fail("Native action must not run"); });
  assert.equal(canActOnArtifact("static-preview", project, artifact), false);
  for (const action of ["open", "reveal"] as const) {
    assert.equal(await actOnArtifact("live-local", project, artifact, action), "unavailable");
    Object.assign(globalThis, { isTauri: true });
    assert.equal(await actOnArtifact("static-preview", project, artifact, action), "unavailable");
    Object.assign(globalThis, { isTauri: false });
  }
});

test("live artifact actions send only the approved command, alias and relative path", async () => {
  Object.assign(globalThis, { isTauri: true });
  const commands: string[] = [];
  mockIPC((command, args) => {
    commands.push(command);
    assert.deepEqual(args, { project_alias: "example", relative_path: artifact.relative_path });
    return command === "open_ghost_artifact" ? "opened" : "revealed";
  });
  assert.equal(canActOnArtifact("live-local", project, artifact), true);
  assert.equal(await actOnArtifact("live-local", project, artifact, "open"), "opened");
  assert.equal(await actOnArtifact("live-local", project, artifact, "reveal"), "revealed");
  assert.deepEqual(commands, ["open_ghost_artifact", "reveal_ghost_artifact"]);
});

test("rejected actions select calm feedback and IPC errors never leak internal details", async () => {
  Object.assign(globalThis, { isTauri: true });
  mockIPC(() => "rejected");
  const state = await actOnArtifact("live-local", project, artifact, "open");
  assert.equal(actionMessages[state], "Action rejected. This artifact is no longer available or allowed.");
  for (const response of [undefined, "opened", { error: "/private/secret-marker" }]) {
    mockIPC(() => response);
    assert.equal(await actOnArtifact("live-local", project, artifact, "reveal"), "unavailable");
  }
  mockIPC(() => { throw new Error("/private/secret-marker"); });
  assert.equal(await actOnArtifact("live-local", project, artifact, "open"), "unavailable");
  assert.ok(!Object.values(actionMessages).join().includes("secret-marker"));
});

test("unsafe, unlisted or unavailable artifact selections cannot invoke actions", async () => {
  Object.assign(globalThis, { isTauri: true });
  mockIPC(() => { assert.fail("Invalid selection must not invoke"); });
  for (const path of ["/tmp/file.md", "../.env", "drafts/context-packs/../secret.md", "drafts/context-packs/.ENV.local.md",
    "drafts/context-packs/file.md.exe", "drafts/context-packs/file.md\0", "drafts/context-packs/file.md\n",
    "drafts/unknown/file.md", "drafts/handoffs/unknown/file.md", "src/App.tsx", "outputs/codex/file.md",
    "drafts\\context-packs\\file.md", "drafts/context-packs/%2e%2e.md", "drafts/context-packs/unlisted.md"]) {
    assert.equal(await actOnArtifact("live-local", project, { ...artifact, relative_path: path }, "open"), "rejected", path);
  }
  assert.equal(await actOnArtifact("live-local", undefined, artifact, "open"), "rejected");
  assert.equal(await actOnArtifact("live-local", { ...project, workspace_exists: false }, artifact, "open"), "rejected");
});

test("all supported recent artifact categories have eligible relative paths", () => {
  for (const path of [artifact.relative_path, "drafts/next-steps/next.md", "drafts/update-packs/pack/README-update.md",
    "drafts/handoffs/codex/handoff.md", "drafts/handoffs/chatgpt/handoff.md", "drafts/handoffs/gemini/handoff.md",
    "drafts/handoffs/antigravity/handoff.md", "outputs/codex/20260911T123456000001Z-codex-output-00000001.md",
    "outputs/terminal/20260911T123456000001Z-terminal-output-00000001.md"]) assert.ok(safeArtifactPath(path), path);
});
