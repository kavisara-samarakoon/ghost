import assert from "node:assert/strict";
import { afterEach, test } from "node:test";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { prepareActionRequest, saveActionRequest, type PreparedRequest } from "../src/action-requests.ts";

Object.assign(globalThis, { window: globalThis, isTauri: false });
afterEach(() => { clearMocks(); Object.assign(globalThis, { isTauri: false }); });
const reviewed: PreparedRequest = {
  id: "123-456", created_at: "2026-09-13T12:00:00Z", action_type: "start_session",
  project_alias: "example", payload: { goal: "Review local work" }, status: "pending",
  preview_title: "Start session request", preview_body: "Project: example\nGoal: Review local work",
  safety_notice: "Request only. No workflow changes have been made.",
};

test("browser and unconfirmed requests never invoke storage", async () => {
  mockIPC(() => assert.fail("No storage call is allowed"));
  await assert.rejects(prepareActionRequest("example", { action_type: "generate_next_steps", payload: {} }));
  await assert.rejects(saveActionRequest(reviewed, true));
  Object.assign(globalThis, { isTauri: true });
  await assert.rejects(saveActionRequest(reviewed, false));
  await assert.rejects(prepareActionRequest("../example", { action_type: "generate_next_steps", payload: {} }));
});

test("preparation only requests a preview; explicit save sends exactly the reviewed content", async () => {
  Object.assign(globalThis, { isTauri: true });
  const calls: unknown[] = [];
  const saved = { path: "/local/ghost/action-requests/request.json", audit_recorded: true };
  mockIPC((command, args) => {
    calls.push({ command, args });
    if (command === "prepare_ghost_action_request") return reviewed;
    assert.equal(command, "save_ghost_action_request");
    return saved;
  });
  const action = { action_type: "start_session" as const, payload: reviewed.payload };
  const preview = await prepareActionRequest("example", action);
  assert.deepEqual(preview, reviewed);
  assert.deepEqual(calls, [{ command: "prepare_ghost_action_request", args: { project_alias: "example", action } }]);
  assert.deepEqual(await saveActionRequest(preview, true), saved);
  assert.deepEqual(calls[1], { command: "save_ghost_action_request", args: { request: reviewed, confirmed: true } });
});

test("raw IPC errors never expose paths or body values; partial audit status stays explicit", async () => {
  Object.assign(globalThis, { isTauri: true });
  mockIPC(() => { throw new Error("/private/secret sensitive body"); });
  await assert.rejects(saveActionRequest(reviewed, true), error => {
    assert.ok(error instanceof Error);
    assert.ok(!error.message.includes("/private/secret"));
    assert.match(error.message, /Check recent requests/);
    return true;
  });
  mockIPC(() => ({ path: "/local/request.json", audit_recorded: false }));
  assert.equal((await saveActionRequest(reviewed, true)).audit_recorded, false);
});
