import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const config = JSON.parse(readFileSync(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8"));

test("production CSP permits only bundled resources, local audio and Tauri IPC", () => {
  const csp = new Map<string, string[]>(config.app.security.csp.split(";").map((part: string) => {
    const [name, ...values] = part.trim().split(/\s+/);
    return [name, values];
  }));
  for (const name of ["default-src", "script-src", "style-src", "img-src"]) {
    assert.deepEqual(csp.get(name), ["'self'"]);
  }
  assert.deepEqual(csp.get("media-src"), ["'self'", "blob:"]);
  assert.deepEqual(csp.get("connect-src"), ["ipc:", "http://ipc.localhost"]);
  for (const name of ["object-src", "frame-src", "base-uri", "form-action"]) {
    assert.deepEqual(csp.get(name), ["'none'"]);
  }
  assert.doesNotMatch(config.app.security.csp, /\*|unsafe-eval|unsafe-inline|openai|https:/);
  assert.equal(config.app.withGlobalTauri, undefined);
  assert.equal(config.app.security.dangerousRemoteDomainIpcAccess, undefined);
});

test("main-window capability surface grants no generic shell, HTTP, filesystem or remote access", () => {
  const capability = JSON.parse(readFileSync(new URL("../src-tauri/capabilities/default.json", import.meta.url), "utf8"));
  assert.deepEqual(capability.windows, ["main"]);
  assert.deepEqual(capability.permissions, ["core:default", "ghost-local-artifacts", "ghost-controlled-input", "ghost-google-assistant", "ghost-personal-memory"]);
  assert.equal(capability.remote, undefined);
  const permissionText = ["ghost-local-artifacts.toml", "ghost-controlled-input.toml", "ghost-google-assistant.toml", "ghost-personal-memory.toml"]
    .map(name => readFileSync(new URL(`../src-tauri/permissions/${name}`, import.meta.url), "utf8")).join("\n");
  assert.doesNotMatch(permissionText, /plugin-shell|plugin-http|plugin-fs|apply_|run_orchestration/);
  assert.match(permissionText, /execute_google_mutation/);
});

test("bundle keeps established identifier, alpha version and microphone-only privacy description", () => {
  assert.equal(config.identifier, "com.kavisara.ghost");
  assert.equal(config.productName, "GHOST");
  assert.equal(config.version, "0.5.0-alpha");
  const plist = readFileSync(new URL("../src-tauri/Info.plist", import.meta.url), "utf8");
  assert.match(plist, /NSMicrophoneUsageDescription/);
  assert.doesNotMatch(plist, /NSCameraUsageDescription/);
  assert.equal((plist.match(/<key>/g) ?? []).length, 1);
});

test("release versions agree across CLI, desktop, Tauri and Cargo sources", () => {
  const packageJson = JSON.parse(readFileSync(new URL("../package.json", import.meta.url), "utf8"));
  assert.equal(packageJson.version, config.version);
  const cargo = readFileSync(new URL("../src-tauri/Cargo.toml", import.meta.url), "utf8");
  const cargoPackage = cargo.split("[package]\n")[1].split("\n[")[0];
  assert.match(cargoPackage, /^name = "desktop"$/m);
  assert.equal(cargoPackage.match(/^version = "([^"]+)"$/m)?.[1], config.version);
  const lock = readFileSync(new URL("../src-tauri/Cargo.lock", import.meta.url), "utf8");
  const desktopEntries = lock.split("[[package]]\n").filter(entry => /^name = "desktop"$/m.test(entry));
  assert.equal(desktopEntries.length, 1);
  assert.equal(desktopEntries[0].match(/^version = "([^"]+)"$/m)?.[1], config.version);
  const cli = readFileSync(new URL("../../cli/src/ghost_cli/__init__.py", import.meta.url), "utf8");
  assert.equal(cli.match(/^__version__ = "([^"]+)"$/m)?.[1], "0.5.0a0");
});
