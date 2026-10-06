# GHOST M39 — Public Repository Privacy & Secret Audit

Audit date: 2026-10-06. Evidence is a point-in-time audit, not a guarantee that
every possible secret or future publication will be detected. Candidate values
are omitted; fingerprints identify findings without reproducing credentials.

## 1. Scope

Initial working tree was clean. Branch: `feature/m39-public-privacy-audit`.
Expected base, initial HEAD and final HEAD:
`a70834eb993efc4294f4be65ec0ff5fcc509b7b8`. Work stops uncommitted. Scope is
repository maintenance, ignore rules, CI and documentation; neither application
runtime, versions, approved branding nor historical release documents changed.

[GitHub repository](https://github.com/kavisara-samarakoon/ghost) was verified
**public**, default branch `main`. Description: “Secure local-first workflow
assistant with a macOS desktop cockpit and Python CLI.” Homepage is unset; topics
are desktop-app, ghost, local-first, macos, python, react, rust, tauri, typescript
and workflow-assistant. Seven GitHub branch tips and five peeled tag commits match
the local public-ref snapshot; no fetch or ref update was needed.

Five releases exist: v0.1.0 (no assets) and prereleases v0.2.0-alpha through
v0.5.0-alpha. The latest prerelease was published 2026-10-05T19:07:27Z;
its tag resolves to the base above. Release assets were downloaded solely to the
temporary audit directory, inspected without execution, detached and removed
after recording evidence. Local redacted evidence remains outside the checkout
under `/private/tmp/ghost-m39/`.

Safety: no real `~/.ghost` contents, registered project records, environment
secret values, or ignored private file contents were read. No OpenAI request,
discovered-credential verification, scanner upload, GitHub write, history rewrite,
force push, tag/release deletion, settings mutation, commit, push, PR, merge, tag,
release publication or deployment occurred. Release-helper tests use fake Git/gh.

## 2. Public/private data model

Publishing the engine does not publish the user's assistant memory. Private
content is not required to build, test or distribute GHOST. Public schemas,
variable names, provider names and clearly synthetic fixtures are safe.

| Data class | Expected location | Source control / ignores | Audit/log treatment | Release/public docs |
| --- | --- | --- | --- | --- |
| Engine, tests, contracts, docs, approved assets | Tracked repository | Public; synthetic fixtures only | No personal input needed | Engine binaries and sanitized docs/examples may be public |
| `GHOST_HOME`, fallback `~/.ghost`, global config/registry | Local selected home; config.yaml / projects.yaml | Private; normal home outside checkout; `.ghost/`, ghost-home/ ignored at any depth | Global events may include project aliases/paths; logs remain private | Never bundle records; public docs mention location/schema/defaults only |
| Global CLI/desktop audits | Home audit.jsonl, request-execution-audit.jsonl, desktop-action/voice/intent-audit.jsonl | Private; `.ghost/` and `*.jsonl` ignored | Identifiers, hashes, counts, outcomes; sensitive metadata keys redacted | No real log in source or published bundle |
| Pending Action Requests and request lifecycle | Home action-requests/ and lifecycle storage | Private under home; custom home outside checkout | Content-free request ID/action/alias/outcome | Real goal/note payloads stay private; contracts synthetic |
| Inert intent plans | Home intent-plans/ | Private under home; never execution approval | Plan IDs, hashes, counts/action types only | No real plan bundled; version/steps schema public |
| Project identity, status, decisions, milestones | `<project>/.ghost/` YAML/Markdown | Private; root/nested `.ghost/` ignored in this repository | Registration path/alias can appear locally | No user record required; schemas and examples public |
| Sessions, goals, notes | Project .ghost/sessions/, active-session.yaml | Private local plaintext | Session audits exclude goal/note text; retain IDs/counts | No real private sessions/notes found in text or bundles; artwork caveat in §11 |
| Context packs, handoffs, next steps, update packs | Project .ghost/drafts/ | Private local drafts, not publication approvals | Relative draft paths/provider/IDs/counts; no excerpts | Sanitized examples only; review any export before sharing |
| Supplied outputs and indexes | Project .ghost/outputs/ | Private; recognized secrets sanitized before storage | IDs/types/redaction flag, not raw output | Never package personal outputs |
| AI task/context/response drafts; provider metadata | M33 review in memory; .ghost/drafts/ai/openai/ | Reviewed outbound content; sanitized advisory draft private on disk | Model/provider, byte limit/count, request hash, relative draft path; no raw response or task | Explicit confirmed provider transmission is off-device, separate from public GitHub |
| Voice audio and transcript | Desktop memory only in current implementation | Not persisted by voice path; recordings/ and transcripts/ exports ignored | Format/duration/byte counts only | Reviewed audio sent only after Send; no transcript/audio bundled |
| Typed intent and proposals | Memory until explicit plan save | Private; saved steps may contain goal/note prose | Input/proposal/plan hashes and bounded metadata; no raw intent/proposal | Explicit confirmed interpretation sends sanitized intent, not project context |
| `OPENAI_API_KEY` | Process environment, Python/native lookup only | Value never tracked or persisted; env files ignored, no `.env` loader | Header-only credential; explicit echoed-key sanitation and fixed errors | Variable name is DOCUMENTED VARIABLE NAME — SAFE; value forbidden |
| Future OAuth/access/refresh tokens, client secrets, cookies/session identifiers | Future private credential storage; no connector implementation here | Private; not fixtures/config values in public source | Must not enter audit/error/frontend content | Only schema/name may be public; no future feature added |
| Future Gmail/Calendar/contacts and personal integration configuration | Future private local storage, not repository | Private; arbitrary custom locations need separate ignore policy | Minimized metadata only; no promised implementation | Personal integration content never belongs in bundles or examples |

Ignore rules are repository-specific and cannot protect another registered
project or an arbitrary custom `GHOST_HOME` name. Keep custom homes outside the
checkout; add `.ghost/` to each other project's policy before use. Plaintext local
workflow records are intentionally private; heuristic redaction is incomplete.

## 3. Current tracked-tree scan

Baseline inventory: **163 tracked paths**: source 69, tests 25, docs 14, images/
icons 19, contracts 11, configuration/lockfiles 17, CI 1, release shell helpers 7.
Every path was classified. JSON is native/frontend configuration or the eleven
synthetic contracts; no user-state YAML, database, audit, env or key file is
tracked. Suspicious auth/session/source names are implementation/tests, not
session dumps. There are 144 text files and 19 binary image/icon files.

Methods: tracked-only HEAD export (never scanning ignored live directories),
installed **gitleaks 8.30.1** directory scan, and an independent stdlib Python
pattern scan. Gitleaks used stock rules via explicit temporary config,
`--redact=100`, `--ignore-gitleaks-allow`, no ignore file, JSON reports and a
300-second timeout. No scanner was installed; no online verification/upload was
enabled. The fallback covered OpenAI/GitHub/AWS/Google/Slack/Stripe prefixes,
private-key headers, JWT, literal bearer/credential assignments and webhook URLs;
names and environment lookups alone are not findings.

Gitleaks returned exit 1: **6 detections**, all synthetic adversarial fixtures
(five private-key detections, including duplicate header cases, and one generic
assignment). This is a reviewed finding result, not a zero-detection claim.
The independent scan found **27 credential-shaped occurrences / 16 distinct
type+fingerprint candidates**. All are expected synthetic tests or the explicitly
fake offline demo. No confirmed credential, private GHOST record, transcript or
audio file was found. Candidate prefixes are omitted entirely.

| Type | Current file:line (representative) | SHA-256 of candidate | Classification |
| --- | --- | --- | --- |
| bearer-literal | `apps/cli/src/ghost_cli/demo.py:34` | `6c49fc5691de8a0246e81afa7223fe165d432911e5ba7cf9b5214f531642bfc9` | EXPECTED TEST / EXAMPLE VALUE — SAFE |
| bearer-literal | `apps/cli/tests/test_action_requests.py:180` | `fb7e36ab9aa3c1a2853d6956211c3a23350490e8810e4157fbb48eaf4635652a` | EXPECTED TEST / EXAMPLE VALUE — SAFE |
| credential-assignment | `apps/cli/tests/test_action_requests.py:256` | `613768192230c6ef19ddb4eebfbc397e95b2bfc5cbf47e07b1c8e6624706d672` | EXPECTED TEST / EXAMPLE VALUE — SAFE |
| openai | `apps/cli/tests/test_ai.py:24` | `1800ea80ae309e9af80ebe15fc2c685a3633d2c88c03fd97ea1dbc45e0df17e4` | EXPECTED TEST / EXAMPLE VALUE — SAFE |
| private-key | `apps/cli/tests/test_ai.py:726` | `3021d90eb9437b2d8f30e8363695c4418b5e5f1870801b5c317e9398ee0f572d` | EXPECTED TEST / EXAMPLE VALUE — SAFE |
| credential-assignment | `apps/cli/tests/test_audit.py:36` | `e564b4081d7a9ea4b00dada53bdae70c99b87b6fce869f0c3dd4d2bfa1e53e1c` | EXPECTED TEST / EXAMPLE VALUE — SAFE |
| openai | `apps/cli/tests/test_context_handoffs.py:157` | `7633490ec86af70528b72b2c5c3b288e6dcbe6e6147c9b966c9ef816f1564a46` | EXPECTED TEST / EXAMPLE VALUE — SAFE |
| github | `apps/cli/tests/test_context_handoffs.py:158` | `264b1a149117f8429cc41677e920064ce41e35e6c1465f62b97a1621aef2be0d` | EXPECTED TEST / EXAMPLE VALUE — SAFE |
| private-key | `apps/cli/tests/test_context_handoffs.py:161` | `8bcac7908eb950419537b91e19adc83ce2c9cbfdacf4f81157fdadfec11f7017` | EXPECTED TEST / EXAMPLE VALUE — SAFE |
| jwt | `apps/cli/tests/test_context_handoffs.py:159` | `7dc2cade04c1d88e37bb0f544f092e9bc3aca6585dabd84f09e49853a23216c6` | EXPECTED TEST / EXAMPLE VALUE — SAFE |
| bearer-literal | `apps/cli/tests/test_context_handoffs.py:154` | `bd835450997dfc19d3b9a9c19e971dd15d951285acef0f7dd452916e26c8a863` | EXPECTED TEST / EXAMPLE VALUE — SAFE |
| bearer-literal | `apps/cli/tests/test_context_handoffs.py:175` | `5d5dcd1f8adcbfaa997aa9170e3f9aa65ad7590604fd0c7792cdd134f8c7bbc0` | EXPECTED TEST / EXAMPLE VALUE — SAFE |
| credential-assignment | `apps/cli/tests/test_context_handoffs.py:148` | `7029e9ed10fe929f80c31a0b9e5720e8dce315c3b9c24dc5733b442232d10c93` | EXPECTED TEST / EXAMPLE VALUE — SAFE |
| credential-assignment | `apps/cli/tests/test_context_handoffs.py:175` | `715dc8493c36579a5b116995100f635e3572fdf8703e708ef1a08d943b36774e` | EXPECTED TEST / EXAMPLE VALUE — SAFE |
| bearer-literal | `apps/cli/tests/test_update_packs.py:163` | `076a0305a1dbd743a32949abd491b070129a2c877edf754b7fb8014a7fb87914` | EXPECTED TEST / EXAMPLE VALUE — SAFE |
| openai | `apps/desktop/src-tauri/src/intent/tests.rs:1302` | `5594db1f77c3d4aba3f5771fa8e53b813fd380794040ea073f385956f659026e` | EXPECTED TEST / EXAMPLE VALUE — SAFE |

Evidence for classification: FAKE_KEY/mock transport in test_ai; deliberately
constructed redaction inputs with raw-/fake/repeating example markers and a
non-cryptographic PEM body in test_context_handoffs; rejection/echo-safety tests
in action-request/intent/audit tests; FAKE-private-material in update-pack tests;
and the labelled DEMO / SAMPLE string in demo.py. Merely being under tests was
not treated as sufficient proof. Good adversarial fixtures were preserved.

## 4. Public Git history scan

Public scope was the union of these 13 local refs, validated against current
GitHub branch/tag tips; origin/HEAD aliases main:

- `refs/remotes/origin/HEAD`
- `refs/remotes/origin/feature/cli-milestone-1`
- `refs/remotes/origin/feature/command-space-shell`
- `refs/remotes/origin/feature/context-handoff-generators`
- `refs/remotes/origin/feature/output-logger-next`
- `refs/remotes/origin/feature/release-helper-scripts`
- `refs/remotes/origin/feature/session-manager`
- `refs/remotes/origin/main`
- `refs/tags/v0.1.0`
- `refs/tags/v0.2.0-alpha`
- `refs/tags/v0.3.0-alpha`
- `refs/tags/v0.4.0-alpha`
- `refs/tags/v0.5.0-alpha`

`git rev-list --objects <refs>` plus batch object type/size inspection yielded
**788 unique objects: 75 commits, 5 annotated tags, 301 trees, 407 blobs**.
Every deduplicated text blob was inspected: 372 textual blobs; 35 image/icon
binary blobs received local printable-string scanning. No blob exceeded the
10 MiB fallback bound; **zero size skips**. All 165 historical nonempty path
names were inspected separately (the empty log separator is not a filename).
No historical env/key/GHOST-state/log/database/transcript/recording filename was
found. Historical binary content is not executed or checked out.

Gitleaks scanned the same public refs in Git patch mode and reported 46 scanned
patch commits / about 1.60 MB, with **5 detections**, all synthetic:
test_action_requests.py:246 at `1260b3df90b0aa20ebb47901b7079393f22dcb26`;
test_ai.py:726 at `918d64dcdd383b9fa0e97ff3c1763d622c8bd2f5` (two header
detections); test_context_handoffs.py:161 and :291 at
`02a9a02ec9883369b8e463259861fbcffb092407`. All are reachable from origin/main.
Patch-mode counts do not imply that all 75 commit snapshots were independently
scanned by gitleaks; the independent reachable-blob scan supplies that coverage.

The fallback found **43 credential-shaped occurrences** across blob versions,
with the same 16 candidate fingerprints as §3; no extra historical secret
candidate. Commit messages and annotated-tag contents were also pattern-scanned.
No credential/private workflow exposure was identified. M39 has no local-only
commits: its HEAD is still the base; its pending edits are outside public history.

## 5. Commit/tag identity metadata

75 commits expose two human names and one service identity, with four distinct
name/email pairs. Addresses are redacted here to avoid amplifying exposure.

| Identity | Redacted public email | Email SHA-256 | Classification |
| --- | --- | --- | --- |
| Kavisara Samarakoon | wmkk…@students.nsbm.ac.lk | 1171d398d2640afb574102cba34e8b12cd9d9909720e8dbde4a643366e53f2d9 | PUBLIC PERSONAL IDENTIFIER — OWNER DECISION |
| GitHub | noreply@github.com | 3c205d8fc749f72977b9331e3179773c315bb1f4860c366de2abe9ec9337730b | GitHub service identity — intentional public |
| Sachini Hansani | hsha…@students.nsbm.ac.lk | a0967d4ea264684ee0f73fd9bf685c21300f558f47c040f290269ffd89975f74 | PUBLIC PERSONAL IDENTIFIER — OWNER DECISION |
| Kavisara Samarakoon | kavi…@gmail.com | 855aaf3ec5cbabaa96efd5a14e1612e346d84299153cce30aa2de8df98ce5bf8 | PUBLIC PERSONAL IDENTIFIER — OWNER DECISION |

The owner name Kavisara Samarakoon and GitHub username kavisara-samarakoon are
INTENTIONAL PUBLIC IDENTITY under the project instructions. Other human name/
email publication remains an owner/contributor decision, not proof of consent.
Author counts: owner student address 47; owner personal address 17; second
contributor student address 11. Committer counts: GitHub service 36; owner
student address 11; owner personal address 17; second contributor 11.

Annotated tag v0.1.0 exposes the second contributor's student address; v0.2.0-alpha,
v0.3.0-alpha, v0.4.0-alpha and v0.5.0-alpha expose the owner's student address.
Representative public commits: owner student identity at the base; owner
personal and second-contributor identities can be located from the email
fingerprints in Git metadata. All five existing annotated tags remain unchanged.
Use an appropriate GitHub noreply or intentionally public professional address
for future commits/tags. Existing metadata is historical public exposure; this
milestone does not rewrite it or promise removal from clones/caches/forks.

## 6. PII/local-path findings

| Finding/type | Classification | Public location / ref | Redacted evidence | Remediation status |
| --- | --- | --- | --- | --- |
| Student/personal email metadata | PUBLIC PERSONAL IDENTIFIER — OWNER DECISION; HISTORICAL PUBLIC EXPOSURE — REVIEW REQUIRED | Commit/tag metadata in §5; origin/main and published tags | Redacted addresses and SHA-256 fingerprints in §5 | Owner decision; future address hygiene recommended; no history rewrite |
| Historical owner-machine build path | LOCAL MACHINE PATH / IDENTIFIER — MEDIUM | docs/release-readiness-m37.md:162,164; introduced at `daf5c0286bbad3208125225e60888ef057797b41`, retained at base/v0.5.0-alpha | `/Users/<user>/IdeaProjects/ghost/.../bundle/`; hashes `dedbfca75748e56f8ef885a45aa40b8e1950a22ad12a02ae0c23cfb47f1629cf`, `11c7a214b4c91e8317c0b6a3b95bc502eeef5370a70a71f4823a257730e0e0d1` | Preserved historical evidence per M39 instructions; owner decision about future neutral paths |
| Launch artwork local path and development-session labels | LOCAL MACHINE PATH / IDENTIFIER — MEDIUM; HISTORICAL PUBLIC EXPOSURE — REVIEW REQUIRED | docs/assets/ghost-v0.3.0-alpha-launch.png, current README image and public history | GHOST-only development path/session identifier and generic dogfooding label; no screenshot text copied into report | Owner must review small UI labels/provenance; approved asset unchanged |
| Dependency build paths in releases | LOCAL MACHINE PATH / IDENTIFIER — MEDIUM; RELEASE-ASSET EXPOSURE — REVIEW REQUIRED | Four published executables, §12 | `/Users/<user>/.cargo/registry/src/...`; representative path hash `3362c428ae1c64d5cb809a927502ac24c8715b724f625191cc8b6d57bc789f5f` | Existing releases unchanged; future path-remapping hardening is an owner decision |
| `/Users/example/...`, `/home/project` | EXPECTED TEST / EXAMPLE VALUE — SAFE | snapshot/tests.rs:159,163,167; apps/cli/README.md:98; historical versions | Synthetic/default/example paths | Retained |
| Icon filename resembling email; SSH Git origin | FALSE POSITIVE | tauri.conf.json:33; release-helper tests:541–542 and history | `128x128@2x.png`; `git@github.com` is SSH syntax | Retained |

No phone-number, postal/street-address, student/account-ID marker, private project
export or local device-name exposure was identified in the bounded text scan.
Public repository names/sample project names are not automatically removed.
PII patterns are heuristic and do not prove absence of all personal information.

## 7. Ignore-policy results

Initial gaps: `.env.production`, `.env.development`, `.env.test`, `.ENV` and
`.ENV.production` were not covered. Existing root/nested `.ghost/` protection was
already correct. Classification: **PRIVACY-HYGIENE GAP — FIX REQUIRED**, fixed.

Root policy now covers all casing variants of `.env` and `.env.*`, with no
example exceptions. No real `.env` was created. It also protects common key
containers, JSONL/log/database exports, ghost-home/, interrupted workspace staging,
and transcript/recording directories. Nested desktop/Tauri rules remain intact.
The case-insensitive index guard catches uppercase private-container names too;
suffix ignores other than env retain their literal lowercase patterns.

All **36 synthetic probes** passed via `git check-ignore --no-index -v`:

| Synthetic path | Ignored | Responsible rule |
| --- | --- | --- |
| `.ghost/private.json` | yes | `.gitignore:55:.ghost/` |
| `nested/project/.ghost/session.json` | yes | `.gitignore:55:.ghost/` |
| `.ghost-dev/test.json` | yes | `.gitignore:52:.ghost-dev/` |
| `.env` | yes | `.gitignore:16:.[eE][nN][vV]` |
| `.env.local` | yes | `.gitignore:17:.[eE][nN][vV].*` |
| `.env.production` | yes | `.gitignore:17:.[eE][nN][vV].*` |
| `.env.development` | yes | `.gitignore:17:.[eE][nN][vV].*` |
| `.env.test` | yes | `.gitignore:17:.[eE][nN][vV].*` |
| `.ENV` | yes | `.gitignore:16:.[eE][nN][vV]` |
| `.ENV.production` | yes | `.gitignore:17:.[eE][nN][vV].*` |
| `secrets.log` | yes | `.gitignore:39:*.log` |
| `debug.log` | yes | `.gitignore:39:*.log` |
| `apps/cli/dist/example.whl` | yes | `.gitignore:6:dist/` |
| `apps/desktop/dist/index.html` | yes | `apps/desktop/.gitignore:11:dist` |
| `apps/desktop/src-tauri/target/release/example` | yes | `apps/desktop/src-tauri/.gitignore:3:/target/` |
| `.DS_Store` | yes | `.gitignore:32:.DS_Store` |
| `.venv/file` | yes | `.gitignore:45:.venv/` |
| `nested/.venv/file` | yes | `.gitignore:45:.venv/` |
| `nested/.env.production` | yes | `.gitignore:17:.[eE][nN][vV].*` |
| `nested/.EnV.test` | yes | `.gitignore:17:.[eE][nN][vV].*` |
| `.env.example` | yes | `.gitignore:17:.[eE][nN][vV].*` |
| `nested/build/file` | yes | `.gitignore:7:build/` |
| `nested/.ghost-setup-example/project.yaml` | yes | `.gitignore:56:.ghost-setup-*/` |
| `ghost-home/projects.yaml` | yes | `.gitignore:57:ghost-home/` |
| `nested/transcripts/example.txt` | yes | `.gitignore:58:transcripts/` |
| `nested/recordings/example.wav` | yes | `.gitignore:59:recordings/` |
| `nested/key.pem` | yes | `.gitignore:20:*.pem` |
| `nested/key.key` | yes | `.gitignore:21:*.key` |
| `nested/key.p12` | yes | `.gitignore:22:*.p12` |
| `nested/key.pfx` | yes | `.gitignore:23:*.pfx` |
| `nested/key.mobileprovision` | yes | `.gitignore:24:*.mobileprovision` |
| `nested/key.cer` | yes | `.gitignore:25:*.cer` |
| `nested/audit.jsonl` | yes | `.gitignore:26:*.jsonl` |
| `nested/state.sqlite` | yes | `.gitignore:27:*.sqlite` |
| `nested/state.sqlite3` | yes | `.gitignore:28:*.sqlite3` |
| `nested/state.db` | yes | `.gitignore:29:*.db` |

All 163 legitimate baseline tracked paths remained unignored and accepted by the
guard; source/test strings naming `.env` or credentials are never content-scanned
by this filename check. The ignore test uses a temporary Git repository copied
only from the three public ignore files. Force-added synthetic state is tested
only there, never in the real index.

## 8. Source credential boundary

Read-only review covered paths/config/workspace/registry, CLI M33, native M35/M36,
frontend adapters, native command registration and production CSP/capabilities.
Credentials come only from Python `os.environ.get("OPENAI_API_KEY")` and native
`std::env::var("OPENAI_API_KEY")`; no environment values were queried during
this audit. No dotenv loader, environment enumeration/dump command or frontend
credential lookup exists. Environment-file content is refused by reviewed CLI
output/task-file paths, rather than loaded as credential configuration.

The key is used in a native/Python request header, never serialized into workflow
state, audit schemas or React. Explicit echoed-key sanitization handles provider
response text. Provider failures produce bounded fixed errors/status information,
not raw bodies or headers. Fixed network paths are api.openai.com/v1/responses
(CLI review/native intent) and /v1/audio/transcriptions (native voice), with no
redirect/proxy/retry/generic endpoint authority. No provider call was made.

Snapshot/search remain offline/read-only; desktop saves remain inert; React CSP
does not permit OpenAI connections. No new confirmed runtime privacy defect was
found; no runtime refactor or feature was introduced. Arbitrary user prose and
identifiers are not guaranteed secret-free: never put secrets in them.

## 9. Audit/log privacy

| Audit | Stored metadata | Excluded content |
| --- | --- | --- |
| CLI global/project | timestamp/event, redacted nested metadata; registration alias/path; session ID/count; output ID/type/redaction; draft relative path/provider | Goal/note content, raw outputs, drafts, API credential/header |
| Action Request / M32 lifecycle | timestamp, action, project alias, request ID, outcome | Payload goal/note and request preview prose |
| AI review | provider/model, output limit/outbound bytes, request SHA, relative saved-draft path, redaction flag | Task/context and provider raw body/key/headers |
| Voice | fixed event/model/MIME category, audio bytes/duration, result, transcript byte count | Audio and transcript |
| Intent | alias, event/result/model, input bytes, request/proposal/plan hashes, schema/outcome/action types/counts/plan ID | Intent, transcript, proposal summary/goal/note text, raw response |
| Orchestration | run ID, alias, plan SHA/count, step index/action type | Full plan prose, goal/note bodies, provider responses |

Audit metadata is not anonymous: aliases, local project paths, identifiers and
hashes can correlate activity. All audits stay private/local. Recursive key-based
redaction is not a universal scanner for secrets embedded in arbitrary safe-key
strings. Intentional workflow state (notes, outputs, AI drafts, saved plan steps)
can contain human prose; it is distinct from content-free event metadata and
never belongs in public Git or release packaging.

## 10. Contract/test-fixture privacy

All seven Action Request contracts and four orchestration contracts contain the
synthetic alias `example`, fixed example timestamps/IDs and generic reviewed-plan
goal/note text. Provider enum names are public. No real path, account ID, note,
credential or registered project state is required.

CLI fixtures isolate both `GHOST_HOME` and Path.home with temporary directories;
native fixtures likewise use generated temporary/synthetic records. Static
preview projects are explicitly presentation-only samples with empty local paths.
Credential-shaped redaction/adversarial fixtures were reviewed by construction
and use, not removed to make scanners green. Runtime suites were not rerun for
repository-only changes.

## 11. Image/asset review

19 current visual/icon files were inventoried with sizes, signatures and PNG/icon
container structures. No JPEG/WEBP/GIF/SVG/PDF or audio file is currently tracked.
PNG metadata was inspected locally via stdlib parsing. EXIF tooling was not
installed; none was downloaded. The three large artwork/source PNGs carry C2PA
`caBX` provenance blocks (21,844 / 23,617 / 21,844 bytes); printable content
identifies OpenAI/generated-media provenance and public signature material, with
no matched credential, home path, email or GPS/EXIF chunk. C2PA is metadata, not
an absence-of-metadata claim. No asset was rewritten or redesigned.

| Tracked asset | Bytes / type | Metadata | Visual review |
| --- | --- | --- | --- |
| `apps/desktop/src-tauri/icons/128x128.png` | 23,050 / .png | IHDR/IDAT/IEND only | Reviewed branding; no personal text |
| `apps/desktop/src-tauri/icons/128x128@2x.png` | 80,356 / .png | IHDR/IDAT/IEND only | Reviewed branding; no personal text |
| `apps/desktop/src-tauri/icons/32x32.png` | 2,423 / .png | IHDR/IDAT/IEND only | Reviewed branding; no personal text |
| `apps/desktop/src-tauri/icons/Square107x107Logo.png` | 17,132 / .png | IHDR/IDAT/IEND only | Reviewed branding; no personal text |
| `apps/desktop/src-tauri/icons/Square142x142Logo.png` | 27,488 / .png | IHDR/IDAT/IEND only | Reviewed branding; no personal text |
| `apps/desktop/src-tauri/icons/Square150x150Logo.png` | 30,223 / .png | IHDR/IDAT/IEND only | Reviewed branding; no personal text |
| `apps/desktop/src-tauri/icons/Square284x284Logo.png` | 97,881 / .png | IHDR/IDAT/IEND only | Reviewed branding; no personal text |
| `apps/desktop/src-tauri/icons/Square30x30Logo.png` | 2,209 / .png | IHDR/IDAT/IEND only | Reviewed branding; no personal text |
| `apps/desktop/src-tauri/icons/Square310x310Logo.png` | 116,058 / .png | IHDR/IDAT/IEND only | Reviewed branding; no personal text |
| `apps/desktop/src-tauri/icons/Square44x44Logo.png` | 4,088 / .png | IHDR/IDAT/IEND only | Reviewed branding; no personal text |
| `apps/desktop/src-tauri/icons/Square71x71Logo.png` | 8,648 / .png | IHDR/IDAT/IEND only | Reviewed branding; no personal text |
| `apps/desktop/src-tauri/icons/Square89x89Logo.png` | 12,651 / .png | IHDR/IDAT/IEND only | Reviewed branding; no personal text |
| `apps/desktop/src-tauri/icons/StoreLogo.png` | 5,015 / .png | IHDR/IDAT/IEND only | Reviewed branding; no personal text |
| `apps/desktop/src-tauri/icons/icon.icns` | 1,978,747 / .icns | Icon image/mask elements only | Reviewed branding; no personal text |
| `apps/desktop/src-tauri/icons/icon.ico` | 97,885 / .ico | 6 icon frames | Reviewed branding; no personal text |
| `apps/desktop/src-tauri/icons/icon.png` | 312,837 / .png | IHDR/IDAT/IEND only | Reviewed branding; no personal text |
| `apps/desktop/src/assets/brand/ghost-app-icon-source.png` | 1,540,711 / .png | C2PA caBX; no EXIF/GPS/text chunks | Reviewed branding; no personal text |
| `apps/desktop/src/assets/brand/ghost-logo.png` | 1,681,538 / .png | C2PA caBX; no EXIF/GPS/text chunks | Reviewed branding; no personal text |
| `docs/assets/ghost-v0.3.0-alpha-launch.png` | 1,783,891 / .png | C2PA caBX; no EXIF/GPS/text chunks | Reviewed; small UI text needs owner review |

All PNGs were visually inspected. ICO/ICNS representative rendered frames were
inspected using local `sips` conversions kept only in the audit directory; not
every individual icon frame was separately inspected. The logo/icon graphics
contain branding, not workflow text. Launch artwork shows a local GHOST project
path, development-session identifier and a generic GHOST dogfooding goal.
No private assistant-life content or credential was identifiable, but generated
provenance does not establish whether every small UI label originated in a real
session. **MANUAL VISUAL PRIVACY REVIEW REQUIRED** for its small labels and
source/consent. This is a documented development-artifact owner decision, not
proof that a private note/session dump is safe to publish. If owner review
identifies actual private session/note content, this gate must be reopened and
blocked. Historical binary blobs received strings inspection, not exhaustive
visual reconstruction of all old variants; C2PA was not fully semantically decoded.

## 12. Published release asset review

Read-only GitHub asset downloads matched API sizes and SHA-256 digests for all
eight assets: each DMG and its `.sha256` sidecar. Sidecar hashes match the DMGs.
DMGs were attached `-readonly -nobrowse -noautoopen`; every app and DMG regular
file was inventoried and inspected with local `strings -a`, plus UTF-16 LE/BE
pattern checks. No file exceeded the 64 MiB release bound; zero skipped files or
mount/scan/detach failures in the completed pass. Applications symlinks were
listed without following them; apps were never executed.

| Tag | Published DMG / bytes | DMG SHA-256 | Inspected scope / finding |
| --- | --- | --- | --- |
| v0.2.0-alpha | `GHOST_0.2.0-alpha_aarch64.dmg` / 2,430,570 | `86108c9c3bc68131e8354b0d38211f21926867ea1a55ab93349cd015505622c8` | All 5 regular files + Applications link; 315 dependency home-path matches; no credential/state |
| v0.3.0-alpha | `GHOST_0.3.0-alpha_aarch64.dmg` / 7,900,890 | `3ae94ed728819d0d5e1c96da6aa309365692624352a2e27f0227ae546083b783` | All 5 regular files + Applications link; 315 dependency home-path matches; no credential/state |
| v0.4.0-alpha | `GHOST_0.4.0-alpha_aarch64.dmg` / 7,961,360 | `fc691e3b24fd70501704134a1e91d470d8e902f3bacce5739ec4535bddc7648c` | All 5 regular files + Applications link; 319 dependency home-path matches; no credential/state |
| v0.5.0-alpha | `GHOST_0.5.0-alpha_aarch64.dmg` / 8,882,911 | `b59f1eee64f4046ee69ee717d44464b1a303389f0877ec19b278d4173bd2a624` | All 5 regular files + Applications link; 474 dependency home-path matches; no credential/state |

Common contents: GHOST.app/Contents/Info.plist, MacOS/desktop, Resources/icon.icns,
DMG .VolumeIcon.icns and .DS_Store; the only link is Applications. No `.ghost`
directory, user-state config, registry, real session/note, transcript, audio,
AI draft or audit file is bundled. Runtime schema/path strings in an executable
are not packaged user records. All four Info.plist bundle identifiers are
`com.kavisara.ghost`, names are GHOST and both version fields match their release
tag without v. Microphone usage description appears only in v0.5.0-alpha.

v0.2/v0.3 sidecars contain a public repository-relative build filename; v0.4/v0.5
use only the DMG basename. No personal path/credential is present in sidecars.
Compiled dependency source paths expose the builder's local username in all
versions: 315/315/319/474 pattern occurrences, all under .cargo/registry, not
registered personal projects. Review-required historical release exposure;
future compiler path remapping is OPTIONAL HARDENING, not performed in M39.

Email-shaped icon strings are FALSE POSITIVE. v0.5's genuine email-shaped string
is a bundled third-party cryptography attribution, not owner/account data
(fingerprint `46434cca93b7ebb58778f471b7f6069230ab252ea6175d60cd2c5afaa51c295f`).
Its one authorization-pattern match is adjacent static executable strings, not a header
credential (fingerprint `1d5e3c1b753c04b4a4f83f09343b9f8463bdd43ca475d3ca005fd551ca0fd5d2`).
`OPENAI_API_KEY` occurs as a literal variable name only in v0.5: DOCUMENTED
VARIABLE NAME — SAFE. No live/possibly live credential was identified.

Limitations: static strings/filenames/metadata inspection does not decompile code,
decode every compressed embedded frontend asset or prove binary reproducibility.
No launched GUI behavior, external provider verification or binary execution was
used to establish privacy. No published release or asset was changed.

## 13. GitHub public-text review

Read-only `gh api` pagination inspected all five release bodies/names, all
37 PR titles/bodies, and all 37 issue-list entries (all PRs; **zero standalone
issues**). Global issue comments and PR review comments returned zero; reviews
for each of the 37 PRs also returned zero. No credential/PII/home-path candidate
matched in this text. Whole conversations were not copied into the report;
public text was scanned in memory. Public source/owner links are intentional.
Future/deleted conversations, attachments and external linked pages are not
included in this point-in-time scope.

Repository metadata reports secret scanning, non-provider patterns, validity
checks and push protection **disabled**. Secret-scanning alerts GET returns HTTP
404 with disabled status: alerts are **UNKNOWN / NOT ACCESSIBLE**, not an
assertion of zero alerts. Private vulnerability reporting GET returned
`enabled: false`; repository security-advisory list returned zero entries.
No settings/rulesets/branch protection were changed. GitHub scanning is not used
as a substitute for the local audit.

SECURITY.md is absent. No verified private security email or enabled private
GitHub reporting channel exists, so a reporting policy claiming such a channel
was not fabricated. Owner follow-up: establish/verify an appropriate private
channel before documenting it. Never post a live credential or private user
data in public issues. Alpha status is not a production-security promise.

## 14. Preventative changes

| Exact file | Purpose |
| --- | --- |
| .gitignore | Close env coverage gaps; ignore common credential containers, private records and local GHOST/voice exports |
| README.md | One active public/private boundary subsection; verified published v0.5 status and source commit |
| .github/workflows/ci.yml | Run stdlib privacy guard/tests in repo-hygiene; lint scripts with existing CLI Ruff dependency |
| scripts/privacy/check_repository.py | Fast deterministic offline Git-index filename guard; no file-content/private-state reads or uploads; hashes even filenames in output |
| scripts/privacy/tests/test_repository_privacy.py | Synthetic ignore, force-add rejection, safe tracked-file compatibility, NUL/path handling and console-redaction regressions |
| docs/public-repository-privacy-m39.md | This evidence/classification/limits/owner-decision report |

The small filename guard intentionally makes **no content-secret detection
claim**. It rejects known state directories, env files, credential containers and
log/database/audit files even when force-added. There is no blanket tests/fixture
exemption and no new third-party runtime dependency. Unknown storage names,
arbitrary personal prose and secrets in otherwise legitimate files still need
manual content/history/release/asset review, such as the local redacted gitleaks
and reachable-blob methods used here. No scanner allowlist was committed to hide
expected tests. Neither CLI nor desktop application files changed.

## 15. Known limitations

- Regex scanners cannot identify all secrets, encodings, addresses, device names
  or private meaning. No provider check proves whether a credential is live.
- All current branch/tag tips matched the public local refs; deleted/unreachable
  refs, GitHub hidden refs, forks, old caches and unpublished local refs are outside
  the object-set scope. No blob was omitted because of the size bound.
- Historical image variants received strings inspection; tiny launch-artwork UI
  labels/provenance still require the owner review described in §11. Icon-frame
  review and C2PA semantics were bounded. No visible private life/state was found.
- Release strings inspection does not exhaustively decode embedded compression or
  reconstruct programs. Completed package inventory found no user-state files or
  critical candidate; this is not a binary reproducibility attestation.
- Ignored files were enumerated by names/categories only, never content-scanned.
  Initial ignored inventory: 28,329 paths, principally native build 23,954,
  virtualenv 3,457, dependencies 842, Python caches 49 and other build/editor/cache
  outputs. No .ghost/.ghost-dev/ghost-home/voice or env record names were observed
  in that inventory. Initial ordinary untracked files: zero. New untracked files
  are only the two privacy scripts and this report; caches remain ignored.
- Custom `GHOST_HOME` locations and other projects' ignore policies are the
  owner's responsibility. An ignore rule cannot remove already public history;
  CI filename prevention is not protection against every arbitrary export name.
- GitHub scanning/reporting settings are disabled; no remote CI run was requested.
  Local validation covers changed maintenance scope, not provider/product runtime.

## 16. Owner decisions required

1. Decide whether both student addresses, the owner's personal address and the
   second contributor's metadata identity should remain public. Coordinate with
   that contributor before any separate historical metadata decision; use a
   chosen public/noreply identity for future commits/tags.
2. Review the historical M37 build paths, launch artwork small development labels
   and release dependency paths. Prefer neutral paths/sanitized captures in future
   evidence. Replacing approved artwork or changing compiler remapping needs a
   separate narrow task; existing history/releases are preserved here.
3. Decide whether to enable GitHub secret scanning/push protection and private
   vulnerability reporting in a separate owner-controlled settings task. Verify a
   private reporting route before publishing SECURITY.md; no email was invented.

No confirmed credential requiring rotation was found. If a live/possibly live
credential is later identified, **rotation/revocation must precede any history-
cleaning decision**. Do not run filter-repo/BFG or rewrite published refs as part
of this milestone. Existing tags/releases retain their provenance; rewriting
would invalidate references and cannot retract other public copies.

## 17. Validation

Completed maintenance validation is recorded below; large runtime regressions
are intentionally excluded because no runtime source changed.

| Exact command/check | Result |
| --- | --- |
| `python3 scripts/privacy/check_repository.py` | PASS; original 163 tracked paths accepted; no content scan implied |
| `python3 -m unittest discover -s scripts/privacy/tests -v` | PASS; 6 tests, including 36 ignore probes and all legitimate tracked paths |
| `apps/cli/.venv/bin/ruff check --config apps/cli/pyproject.toml scripts/privacy` | PASS; same Ruff rules as CI CLI working directory |
| `bash -c 'for script in scripts/release/*.sh; do bash -n "$script" || exit 1; done'` | PASS; all 7 helpers |
| `python3 -m unittest discover -s scripts/release/tests -v` | PASS; 59 tests with fake Git/gh, no repository mutations |
| CI YAML: `apps/cli/.venv/bin/python` + PyYAML safe_load/assertions | PASS; 5 existing jobs, contents:read, new privacy steps and lint step verified |
| `git check-ignore --no-index -v <path>` for §7 synthetic paths | PASS; all 36, no real secret file created |
| gitleaks 8.30.1 tracked export / explicit public refs, redacted/offline | Completed; reviewed synthetic detections 6 / 5; exits 1 due expected fixtures |
| stdlib current/reachable-object/metadata, GitHub-memory and asset strings scans | Completed, zero size skips; no confirmed credential/private-state finding |
| Four read-only release mounts / API digests / sidecars / UTF-16 checks | PASS; all 8 asset digests, all 4 sidecar hashes, all mounts detached |
| `git diff --check` and final diff/status/name review | PASS; tracked and three new-file whitespace checks; exactly six intended files |

Final modified tracked paths: .github/workflows/ci.yml, .gitignore, README.md.
Final untracked intended deliverables: docs/public-repository-privacy-m39.md,
scripts/privacy/check_repository.py, scripts/privacy/tests/test_repository_privacy.py.
No generated scan report, downloaded release file, temporary audit file, personal
GHOST state, runtime change or version bump is included in the diff.

## 18. Verdict

PUBLIC REPOSITORY PRIVACY GATE: PASS

No confirmed live/possibly live credential or private GHOST memory/state leak was
identified in the audited current tree, reachable public textual history or
published package files. Root/nested personal-state ignores work; confirmed env
hygiene gaps are fixed and guarded in CI. Remaining public identities, historical
development paths and artwork labels are documented owner decisions with explicit
scan limits. This verdict does not establish production security or permission to
publish personal records; owner review discovering actual private artwork content
must reopen the gate. Work remains uncommitted with HEAD/refs/releases unchanged.
