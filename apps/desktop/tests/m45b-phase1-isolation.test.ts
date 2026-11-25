import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync, readdirSync } from "node:fs";
import { test } from "node:test";
import ts from "typescript";
import { adoptTranscript, JarvisClient, noSharing, type JarvisReview } from "../src/jarvis/jarvis.ts";
import { AutomationClient, openAutomationItem, type AutomationItem } from "../src/automations.ts";
import { MAX_COMMAND_BYTES } from "../src/transcript-handoff.ts";

// Phase I-01 freezes production wiring. These are snapshots of actual application
// sources, not claims of native functionality or an execution/security analyser.
// A later authorised integration must deliberately replace these phase guards.
// No Git commands, application services or production writes run in this suite.
const root = new URL("../../../", import.meta.url);
type Surface = Map<string, string>;
const read = (path: string) => readFileSync(new URL(path, root), "utf8");
function files(directory: string): string[] {
  return readdirSync(new URL(directory + "/", root), { withFileTypes: true }).flatMap(entry => {
    const path = directory + "/" + entry.name;
    return entry.isDirectory() ? files(path) : [path];
  }).sort();
}
function surface(paths: string[]): Surface {
  return new Map(paths.map(path => [path, read(path)]));
}

// Tokenise Rust without dropping whitespace inside literals or joining tokens.
// Includes nested comments, raw strings, lifetimes, chars and compound operators.
// This is only a normaliser; the complete production snapshot supplies the guard.
function rustTokens(source: string): string[] {
  const tokens: string[] = [];
  let i = 0;
  while (i < source.length) {
    const rest = source.slice(i);
    if (/^\s/.test(rest)) { i++; continue; }
    if (rest.startsWith("//")) {
      const end = source.indexOf("\n", i);
      i = end < 0 ? source.length : end + 1; continue;
    }
    if (rest.startsWith("/*")) {
      let depth = 1; i += 2;
      while (depth && i < source.length) {
        if (source.startsWith("/*", i)) { depth++; i += 2; }
        else if (source.startsWith("*/", i)) { depth--; i += 2; }
        else i++;
      }
      assert.equal(depth, 0, "unterminated Rust comment"); continue;
    }
    const raw = rest.match(/^(?:br|cr|r)(#*)"/);
    if (raw) {
      const end = source.indexOf('"' + raw[1], i + raw[0].length);
      assert.notEqual(end, -1, "unterminated Rust raw literal");
      const length = end + 1 + raw[1].length - i;
      tokens.push(source.slice(i, i + length)); i += length; continue;
    }
    if (rest[0] === '"') {
      const literal = rest.match(/^"(?:\\[\s\S]|[^"\\])*"/);
      assert.ok(literal, "unterminated Rust string");
      tokens.push(literal[0]); i += literal[0].length; continue;
    }
    const token = rest.match(/^'(?:\\(?:u\{[0-9a-fA-F_]+\}|x[0-9a-fA-F]{2}|.)|[^'\\\n])'|^[A-Za-z_][A-Za-z_0-9]*|^\d[\d_]*(?:\.\d[\d_]*)?(?:[eE][+-]?\d[\d_]*)?(?:[A-Za-z_]\w*)?|^(?:>>=|<<=|\.\.=|\.\.\.|::|->|=>|==|!=|<=|>=|&&|\|\||\+=|-=|\*=|\/=|%=|&=|\|=|\^=|<<|>>|\.\.)/u);
    const text = token?.[0] ?? rest[0];
    tokens.push(text); i += text.length;
  }
  return tokens;
}
function normalise(path: string, source: string): string {
  if (path.endsWith(".rs")) return JSON.stringify(rustTokens(source));
  if (/\.tsx?$/.test(path)) {
    const ast = ts.createSourceFile(path, source, ts.ScriptTarget.Latest, true,
      path.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
    return ts.createPrinter({ removeComments: true, newLine: ts.NewLineKind.LineFeed }).printFile(ast);
  }
  // Python indentation and configuration/lockfile bytes are deliberately exact.
  return source;
}
function fingerprint(input: Surface): string {
  const records = [...input].sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0)
    .map(([path, source]) => [path, normalise(path, source)]);
  return createHash("sha256").update(JSON.stringify(records)).digest("hex");
}
const desktop = "apps/desktop/";
const nativeRoot = desktop + "src-tauri/";
const native = surface(files(nativeRoot + "src").filter(path => path.endsWith(".rs") && !path.includes("/voice_conversation/")));
const frontend = surface(files(desktop + "src").filter(path => /\.tsx?$/.test(path)));
const cli = surface(files("apps/cli/src").filter(path => path.endsWith(".py")));
const wiring = surface([
  ...files(nativeRoot + "capabilities"), ...files(nativeRoot + "permissions"),
  ...readdirSync(new URL(nativeRoot, root), { withFileTypes: true })
    .filter(entry => entry.isFile() && /(?:\.(?:json|json5|toml|plist|entitlements)|Cargo\.lock|build\.rs)$/.test(entry.name))
    .map(entry => nativeRoot + entry.name),
  ...readdirSync(new URL(desktop, root), { withFileTypes: true })
    .filter(entry => entry.isFile() && /(?:package\.json|pnpm-lock\.yaml|tsconfig.*\.json|vite\.config\.[cm]?ts|index\.html)$/.test(entry.name))
    .map(entry => desktop + entry.name),
]);
// Filled from the verified accepted pre-task worktree. lib.rs includes only the
// accepted #[cfg(test)] registration relative to baseline HEAD 24c0a89b.
const accepted = {
  native: "dca77c1bbd1034e50c54513426b13360e39e45934505b3c8370f2f663c9e921e",
  frontend: "83376a7b54d3bd6033fc0336e034e500742f9945951dd63415e58d5591ae5915",
  cli: "24475cdb2845a98886105948c5c9a49d938f72c5b2b85b00e35125a430278624",
  wiring: "7c267d50c24e2a46a300bd456d834cce5c6e9aa6fc8eb32c26ea87acb284bfa2",
};
function verify(input: Surface, expected: string) {
  assert.equal(fingerprint(input), expected, "production surface changed; requires separate authorisation");
}
function mutated(input: Surface, path: string, before: string, after: string): Surface {
  const copy = new Map(input);
  const original = copy.get(path);
  assert.ok(original !== undefined && original.includes(before), "negative control must change a real protected boundary");
  copy.set(path, original.replace(before, after));
  return copy;
}
function rejectsChange(input: Surface, expected: string, path: string, before: string, after: string) {
  // Construct outside assert.throws: a broken fixture must fail, not count as
  // successful detection of a changed boundary.
  const changed = mutated(input, path, before, after);
  assert.throws(() => verify(changed, expected), /production surface changed/);
}

test("M45B root registration is exactly test-only, independent of features and debug mode", () => {
  const tokens = rustTokens(read(nativeRoot + "src/lib.rs"));
  const positions = tokens.flatMap((token, i) => token === "voice_conversation" ? [i] : []);
  assert.equal(positions.length, 1);
  const i = positions[0];
  assert.deepEqual(tokens.slice(i - 8, i + 2), ["#", "[", "cfg", "(", "test", ")", "]", "mod", "voice_conversation", ";"]);
  verify(native, accepted.native);
});
test("native startup, IPC, providers, consent issuers and persistence remain at the accepted production surface", () => verify(native, accepted.native));
test("M44, M45 manual capture, adoption and finite planning frontend sources remain preserved", () => verify(frontend, accepted.frontend));
test("CLI APPLY and plan-hash RUN gates and fixed dispatcher remain preserved", () => verify(cli, accepted.cli));
test("features, dependencies, lockfiles, CSP, capabilities, permissions and native metadata remain preserved", () => verify(wiring, accepted.wiring));

test("source guards tolerate comments and harmless Rust/TypeScript formatting", () => {
  const lib = nativeRoot + "src/lib.rs";
  const changed = mutated(native, lib, "mod voice;", "mod /* harmless */ voice ;");
  changed.set(lib, "// harmless comment\n" + changed.get(lib));
  assert.equal(fingerprint(changed), fingerprint(native));
  const path = desktop + "src/transcript-handoff.ts";
  const formatted = mutated(frontend, path, "MAX_COMMAND_BYTES = 8192;", "MAX_COMMAND_BYTES=8192 ; // harmless\n");
  assert.equal(fingerprint(formatted), fingerprint(frontend));
  assert.notDeepEqual(rustTokens('"private text"'), rustTokens('"privatetext"'));
  assert.notDeepEqual(rustTokens("a b"), rustTokens("ab"));
  assert.deepEqual(rustTokens('/* outer /* inner */ end */ r#"// literal"#'), ['r#"// literal"#']);
});

const nativeControls: [string, string, string, string][] = [
  ["feature-enabled test module", "lib.rs", "#[cfg(test)]\nmod voice_conversation;", '#[cfg(any(test, feature = "voice"))]\nmod voice_conversation;'],
  ["debug-only registration", "lib.rs", "#[cfg(test)]", "#[cfg(debug_assertions)]"],
  ["production fake issuer", "lib.rs", "pub fn run() {", "pub fn issue_fake_consent() {}\npub fn run() {"],
  ["activation IPC", "lib.rs", "tauri::generate_handler![", "tauri::generate_handler![voice_conversation::enable,"],
  ["startup service", "lib.rs", "tauri::Builder::default()", "tauri::Builder::default().manage(VoiceService::start())"],
  ["Google confirmation bypass", "connectors/assistant/runtime.rs", "input.confirmation != saved.confirmation()", "false"],
  ["memory confirmation bypass", "memory/mutations.rs", "input.confirmation != phrase(p.action, &p.request_sha256)", "false"],
];
for (const [name, path, before, after] of nativeControls) test(`negative control detects ${name}`, () => {
  rejectsChange(native, accepted.native, nativeRoot + "src/" + path, before, after);
});
test("negative control detects alternate production module loaders and new source paths", () => {
  for (const source of ['#[path="voice_conversation/mod.rs"] mod activated;', 'include!("voice_conversation/consent.rs");']) {
    const changed = new Map(native);
    changed.set(nativeRoot + "src/activation.rs", source);
    assert.throws(() => verify(changed, accepted.native), /production surface changed/);
  }
});
const frontendControls: [string, string, string, string][] = [
  ["unbounded adoption", "transcript-handoff.ts", "MAX_COMMAND_BYTES = 8192", "MAX_COMMAND_BYTES = 999999"],
  ["automatic conversation planning", "jarvis/jarvis.ts", "client.invalidate();\n  return", 'client.prepare({command: text, project_alias: null, context_query: null, sharing: noSharing});\n  return'],
  ["automation execution", "automations.ts", '"evaluate_automations"', '"execute_automation_action"'],
  ["action request execution status", "action-requests.ts", 'status: "pending"', 'status: "executed"'],
];
for (const [name, path, before, after] of frontendControls) test(`negative control detects ${name}`, () => {
  rejectsChange(frontend, accepted.frontend, desktop + "src/" + path, before, after);
});
test("negative controls detect changed exact CLI confirmations", () => {
  for (const [path, before, after] of [
    ["request_execution.py", 'confirmation != f"APPLY {review.request.id}"', "False"],
    ["orchestration.py", 'f"RUN {self.project.alias} PLAN {self.fingerprint}"', 'f"RUN {self.project.alias}"'],
    ["orchestration.py", "confirmation != prepared.confirmation", "False"],
  ]) rejectsChange(cli, accepted.cli, "apps/cli/src/ghost_cli/" + path, before, after);
});
test("negative controls detect feature, permission, CSP and dependency expansion", () => {
  for (const [path, before, after] of [
    ["Cargo.toml", "[dependencies]", '[features]\nvoice = []\n[dependencies]'],
    ["Cargo.toml", "[dependencies]", '[dependencies]\nnative_voice = "1"'],
    ["capabilities/default.json", '"core:default"', '"core:default", "voice-enable"'],
    ["tauri.conf.json", "ipc: http://ipc.localhost", "ipc: http://ipc.localhost https:"],
  ]) rejectsChange(wiring, accepted.wiring, nativeRoot + path, before, after);
  const platformOverride = new Map(wiring);
  platformOverride.set(nativeRoot + "tauri.macos.conf.json", '{"app":{"security":{"csp":null}}}');
  assert.throws(() => verify(platformOverride, accepted.wiring), /production surface changed/);
});

function review(): JarvisReview {
  return { version: 1, schema_version: 1, command: "Synthetic", project_alias: null, context_query: null,
    sharing: { personal_memory: true, project_memory: true }, model: "synthetic", created_at: 1, expires_at: 301,
    context: [], outbound_input: "synthetic", outbound_bytes: 9, request_sha256: "a".repeat(64), safety_notice: "Synthetic only" };
}
test("explicit transcript adoption is byte-bounded editable draft data and invalidates prior review", async () => {
  const calls: string[] = [];
  const client = new JarvisClient(() => true, async<T>(name: string) => { calls.push(name); return review() as T; });
  const prior = await client.prepare({ command: "Synthetic", project_alias: null, context_query: null, sharing: { personal_memory: true, project_memory: true } });
  const text = "é".repeat(MAX_COMMAND_BYTES / 2);
  const result = adoptTranscript(client, text, false);
  assert.equal(result.error, null); assert.ok(result.draft);
  assert.deepEqual(result.draft, { command: text, sharing: noSharing, query: "", review: null, proposal: null });
  result.draft.command = "Locally edited";
  await assert.rejects(client.send(prior));
  assert.equal(adoptTranscript(client, text + "é", false).draft, null);
  assert.equal(adoptTranscript(client, text, true).draft, null);
  assert.deepEqual(calls, ["prepare_jarvis_request"]);
});
test("automation evaluation and attention navigation issue no planning or action requests", async () => {
  const calls: string[] = [];
  const item: AutomationItem = { version: 1, id: "synthetic-item", automation_id: "synthetic-automation", automation_title: "Synthetic",
    task: { kind: "command_prompt", prompt: "Synthetic inert prompt", project_alias: null }, triggered_at: 1, occurrence_key: "synthetic", status: "pending" };
  const client = new AutomationClient(() => true, async<T>(name: string) => { calls.push(name); return { created: [item], pending_count: 1, audit_recorded: true } as T; });
  const outcome = await client.evaluate();
  const navigation: unknown[] = [];
  openAutomationItem(outcome.created[0], (...args) => navigation.push(args));
  assert.deepEqual(calls, ["evaluate_automations"]);
  assert.deepEqual(navigation, [["Command", { id: item.id, text: "Synthetic inert prompt" }, null]]);
});
