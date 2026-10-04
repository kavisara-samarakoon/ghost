"""Exercise release safeguards with fake git/gh only; no real commits or network calls.

Run: python3 -m unittest discover -s scripts/release/tests -v
"""

import json
import os
import shlex
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

RELEASE = Path(__file__).resolve().parents[1]
HEAD = "a" * 40
MERGE = "b" * 40
URL = "https://github.com/example/ghost/pull/12"


def mock_command(tool, args, state):
    """Small deterministic command doubles; unknown commands fail closed."""
    if [tool, *args] in state.get("fail_calls", []):
        return 1, "Mock command failure"
    if tool == "git":
        command = args[0]
        if command == "rev-parse":
            if "--show-toplevel" in args:
                return (1, "") if state.get("no_repo") else (0, os.environ["MOCK_ROOT"])
            if args[-1] == "refs/remotes/origin/main":
                return 0, state.get("origin_main", HEAD)
            return 0, state.get("local_head", HEAD)
        if command == "symbolic-ref":
            return (1, "") if state.get("detached") else (0, state["branch"])
        if command == "check-ref-format":
            if "--branch" in args and args[-1] == "HEAD":
                return 1, ""
            return (1, "") if any(c in args[-1] for c in "~^:?[**") else (0, "")
        if command == "status":
            if "--porcelain" in args:
                state["status_reads"] = state.get("status_reads", 0) + 1
            dirty = state.get("dirty") or (
                state.get("dirty_on_read", 999) <= state.get("status_reads", 0)
            )
            return 0, " M approved.txt" if dirty else ""
        if command == "remote":
            return 0, state.get("origin", "https://github.com/example/ghost.git")
        if command == "show-ref":
            return (
                (0, "")
                if state.get("local_ref") or args[-1] == "refs/heads/main"
                else (1, "")
            )
        if command == "ls-remote":
            return state.get("remote_code", 2), ""
        if command == "switch":
            state["branch"] = args[-1]
            return 0, ""
        if command == "diff":
            staged = state["staged"]
            if "--cached" in args and "--quiet" in args:
                return (1 if staged else 0), ""
            if "--check" in args:
                return state.get("diff_error", 0), ""
            if "--name-only" in args:
                delimiter = "\0" if "-z" in args else "\n"
                return 0, delimiter.join(staged) + (delimiter if staged else "")
            return 0, ""
        if command == "ls-files":
            return (0, args[-1]) if args[-1] in state.get("deleted", []) else (1, "")
        if command == "add":
            if state.get("add_failure") == args[-1]:
                return 1, ""
            if args[-1] not in state["staged"]:
                state["staged"].append(args[-1])
            if (
                state.get("inject_unapproved")
                and "unapproved.txt" not in state["staged"]
            ):
                state["staged"].append("unapproved.txt")
            return 0, ""
        if command == "restore":
            state["staged"] = [path for path in state["staged"] if path != args[-1]]
            return 0, ""
        if command == "write-tree":
            state["tree_reads"] = state.get("tree_reads", 0) + 1
            return 0, "changed" if state.get("tree_changed") and state[
                "tree_reads"
            ] > 1 else HEAD
        if command == "commit":
            if state.get("commit_error"):
                return 1, "Mock commit failure"
            state["committed"] = True
            state["staged"] = []
            state["dirty"] = state.get("unapproved_dirty", False)
            state["branch"] = state.get("branch_after_commit", state["branch"])
            return 0, "Mock commit only"
        if command == "merge-base":
            return state.get("ancestor_error", 0), ""
        if command == "fetch":
            state["origin_main"] = state.get(
                "origin_after_fetch", state.get("origin_main", HEAD)
            )
            return 0, ""
        if command in ("push", "pull", "fetch", "tag", "merge-base", "log", "branch"):
            return 0, ""
    if tool == "gh":
        if args[0] == "auth":
            return state.get("auth_error", 0), ""
        if args[:2] == ["pr", "create"]:
            state.update(state.get("after_create", {}))
            return state.get("create_error", 0), state.get("created_url", URL)
        if args[:2] == ["pr", "merge"]:
            state["merged"] = True
            return 0, ""
        if args[:2] == ["pr", "view"]:
            if state.get("view_error"):
                return 1, ""
            fields = args[args.index("--json") + 1]
            if fields == "statusCheckRollup":
                return 0, str(state.get("check_count", 1))
            if fields == "state,mergeCommit":
                return 0, "OPEN\t" if state.get(
                    "queued_merge"
                ) else f"MERGED\t{state.get('merge_oid', MERGE)}"
            state["pr_reads"] = state.get("pr_reads", 0) + 1
            state.update(state.get("pr_updates", {}).get(str(state["pr_reads"]), {}))
            oid = (
                "c" * 40
                if state.get("head_changed") and state["pr_reads"] > 2
                else state.get("head_oid", HEAD)
            )
            values = [
                state.get("pr_number", "12"),
                state.get("pr_state", "OPEN"),
                state.get("changed_branch", "feature/changed")
                if state.get("branch_changed") and state["pr_reads"] > 2
                else state.get("head_branch", "feature/example"),
                state.get("base", "main"),
                URL,
                oid,
                "Example milestone",
            ]
            if '"number="' in args[-1]:
                values = [
                    prefix + value
                    for prefix, value in zip(
                        (
                            "number=",
                            "state=",
                            "head=",
                            "base=",
                            "url=",
                            "oid=",
                            "title=",
                        ),
                        values,
                    )
                ]
            return 0, "\t".join(values)
        if args[:2] == ["pr", "checks"]:
            if "--help" in args:
                return 0, "--watch" if not state.get("old_gh") else "checks help"
            if "--watch" in args:
                return state.get("watch_error", 0), ""
            if "--json" in args:
                if state.get("checks_json_error"):
                    return 1, ""
                if state.get("check_count", 1) == 0:
                    return 0, ""
                if (
                    state.get("checks_state_changed")
                    and state.get("check_reads", 0) > 1
                ):
                    return 0, "pass\tFAILURE"
                return 0, state.get("checks", "pass\tSUCCESS")
            state["check_reads"] = state.get("check_reads", 0) + 1
            if state.get("checks_changed") and state["check_reads"] > 1:
                return 8, ""
            return state.get("checks_error", 0), ""
    return 99, f"Unexpected mock call: {tool} {args}"


def run_mock():
    state_path = Path(os.environ["MOCK_STATE"])
    state = json.loads(state_path.read_text())
    tool, args = sys.argv[2], sys.argv[3:]
    state["calls"].append([tool, *args])
    code, output = mock_command(tool, args, state)
    state_path.write_text(json.dumps(state))
    if output:
        sys.stdout.write(output)
        if not output.endswith(("\n", "\0")):
            sys.stdout.write("\n")
    raise SystemExit(code)


class ReleaseScriptsTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="ghost-release-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.state_path = self.root / "mock-state.json"
        self.state_path.write_text(
            json.dumps(
                {
                    "branch": "feature/example",
                    "staged": [],
                    "calls": [],
                    "dirty": False,
                }
            )
        )
        self.bin_dir = self.root / "bin"
        self.bin_dir.mkdir()
        for tool in ("git", "gh"):
            executable = self.bin_dir / tool
            executable.write_text(
                f"#!/bin/bash\nexec {shlex.quote(sys.executable)} "
                f'{shlex.quote(str(Path(__file__).resolve()))} --mock {tool} "$@"\n'
            )
            executable.chmod(0o700)
        (self.root / "approved.txt").write_text("approved change\n")

    def state(self):
        return json.loads(self.state_path.read_text())

    def configure(self, **values):
        state = self.state()
        state.update(values)
        self.state_path.write_text(json.dumps(state))

    def run_script(self, name, args, confirmation=""):
        environment = os.environ.copy()
        environment.update(
            {
                "PATH": f"{self.bin_dir}:{environment['PATH']}",
                "MOCK_ROOT": str(self.root),
                "MOCK_STATE": str(self.state_path),
                "TMPDIR": str(self.root),
            }
        )
        return subprocess.run(
            ["/bin/bash", str(RELEASE / f"{name}.sh"), *args],
            input=confirmation,
            text=True,
            capture_output=True,
            check=False,
            cwd=self.root,
            env=environment,
            timeout=20,
        )

    def assert_not_called(self, tool, command):
        self.assertFalse(
            any(call[:2] == [tool, command] for call in self.state()["calls"])
        )

    def merge(self, confirmation="merge PR #12 and tag v0.2.5\n", tag="v0.2.5"):
        return self.run_script(
            "merge-and-tag",
            ["--pr", "12", "--tag", tag, "--message", "Milestone"],
            confirmation,
        )

    def commit(
        self, files=None, confirmation='commit "Milestone" with approved files\n'
    ):
        return self.run_script(
            "commit-milestone",
            ["--message", "Milestone", "--files", *(files or ["approved.txt"])],
            confirmation,
        )

    def test_start_success(self):
        result = self.run_script("start-milestone", ["feature/new"])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(
            ["git", "pull", "--ff-only", "origin", "main"], self.state()["calls"]
        )
        self.assertEqual(self.state()["branch"], "feature/new")

    def test_unsafe_branch_names(self):
        for name in (
            "",
            "HEAD",
            "main",
            "master",
            "-bad",
            "bad branch",
            "a..b",
            "a@{b",
            "a\\b",
            "a//b",
            "a/",
            "a:b",
        ):
            with self.subTest(name=name):
                self.assertNotEqual(
                    self.run_script("start-milestone", [name]).returncode, 0
                )
        self.assert_not_called("git", "switch")

    def test_start_refuses_dirty_tree_and_ref_collisions(self):
        for settings in (
            {"dirty": True},
            {"dirty": False, "local_ref": True},
            {"local_ref": False, "remote_code": 0},
            {"remote_code": 128},
        ):
            self.configure(**settings)
            self.assertNotEqual(
                self.run_script("start-milestone", ["feature/new"]).returncode, 0
            )
        self.assert_not_called("git", "switch")

    def test_commit_explicit_files_and_deletions(self):
        filenames = [
            "space name.txt",
            "literal*[x].txt",
            "-option.txt",
            "line\nbreak.txt",
            "gone.txt",
        ]
        for name in filenames[:-1]:
            (self.root / name).write_text("changed\n")
        self.configure(deleted=["gone.txt"])
        result = self.commit(filenames)
        self.assertEqual(result.returncode, 0, result.stderr)
        additions = [
            call for call in self.state()["calls"] if call[:2] == ["git", "add"]
        ]
        self.assertEqual(additions, [["git", "add", "--", name] for name in filenames])
        self.assertTrue(self.state()["committed"])

    def test_commit_rejects_nonfile_and_missing_paths(self):
        for path in (
            ".",
            "bin",
            "../outside",
            "/absolute",
            "missing",
            "./approved.txt",
        ):
            self.assertNotEqual(self.commit([path]).returncode, 0)
        self.assert_not_called("git", "add")

    def test_commit_rejects_prestaged_changes_without_touching_index(self):
        self.configure(staged=["unapproved.txt"])
        self.assertNotEqual(self.commit().returncode, 0)
        self.assertEqual(self.state()["staged"], ["unapproved.txt"])
        self.assert_not_called("git", "add")
        self.assert_not_called("git", "restore")

    def test_commit_cancellation_unstages_script_paths(self):
        self.assertNotEqual(self.commit(confirmation="yes\n").returncode, 0)
        self.assertEqual(self.state()["staged"], [])
        self.assertTrue((self.root / "approved.txt").exists())
        self.assert_not_called("git", "commit")

    def test_commit_refuses_unapproved_index_change(self):
        self.configure(inject_unapproved=True)
        self.assertNotEqual(self.commit().returncode, 0)
        self.assertEqual(self.state()["staged"], ["unapproved.txt"])
        self.assert_not_called("git", "commit")

    def test_commit_refuses_changed_staged_content(self):
        self.configure(tree_changed=True)
        self.assertNotEqual(self.commit().returncode, 0)
        self.assertEqual(self.state()["staged"], [])
        self.assert_not_called("git", "commit")

    def test_commit_refuses_main_master_detached_and_no_repo(self):
        for settings in (
            {"branch": "main"},
            {"branch": "master"},
            {"branch": "feature/example", "detached": True},
            {"detached": False, "no_repo": True},
        ):
            self.configure(**settings)
            self.assertNotEqual(self.commit().returncode, 0)
        self.assert_not_called("git", "add")

    def test_commit_diff_failure_prevents_staging(self):
        self.configure(diff_error=2)
        self.assertNotEqual(self.commit().returncode, 0)
        self.assert_not_called("git", "add")

    def test_open_pr_pushes_without_force_and_never_merges(self):
        result = self.run_script(
            "open-pr", ["--title", "Title", "--body", "Line one\nLine two"]
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(
            ["git", "push", "-u", "origin", "feature/example"], self.state()["calls"]
        )
        self.assertIn(URL, result.stdout)
        self.assertFalse(
            any(call[:3] == ["gh", "pr", "merge"] for call in self.state()["calls"])
        )

    def test_open_pr_reports_no_checks_without_claiming_pass(self):
        self.configure(check_count=0)
        result = self.run_script("open-pr", ["--title", "Title", "--body", "Body"])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("checks have NOT passed", result.stdout)
        self.assertFalse(any("--watch" in call for call in self.state()["calls"]))

    def test_open_pr_refuses_dirty_tree(self):
        self.configure(dirty=True)
        self.assertNotEqual(
            self.run_script(
                "open-pr", ["--title", "Title", "--body", "Body"]
            ).returncode,
            0,
        )
        self.assert_not_called("git", "push")

    def test_open_pr_auth_failure_prevents_push(self):
        self.configure(auth_error=1)
        result = self.run_script("open-pr", ["--title", "Title", "--body", "Body"])
        self.assertNotEqual(result.returncode, 0)
        self.assert_not_called("git", "push")

    def test_open_pr_supports_standard_ssh_origin(self):
        for origin in (
            "git@github.com:example/ghost.git",
            "ssh://git@github.com/example/ghost.git",
        ):
            self.configure(origin=origin)
            result = self.run_script("open-pr", ["--title", "Title", "--body", "Body"])
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn(
                "GitHub operations target github.com/example/ghost", result.stdout
            )

    def test_open_pr_watch_failure_reports_url(self):
        self.configure(watch_error=1)
        result = self.run_script("open-pr", ["--title", "Title", "--body", "Body"])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(URL, result.stderr)

    def test_merge_success_pins_head_and_tags_merge_commit(self):
        result = self.merge()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(
            [
                "gh",
                "pr",
                "merge",
                "12",
                "--squash",
                "--delete-branch",
                "--match-head-commit",
                HEAD,
            ],
            self.state()["calls"],
        )
        self.assertIn(
            ["git", "tag", "-a", "v0.2.5", "-m", "Milestone", MERGE],
            self.state()["calls"],
        )
        self.assertIn(
            ["git", "push", "origin", "refs/tags/v0.2.5"], self.state()["calls"]
        )

    def test_merge_refuses_every_non_success_state(self):
        for state in (
            "FAILURE",
            "ERROR",
            "CANCELLED",
            "SKIPPED",
            "PENDING",
            "QUEUED",
            "IN_PROGRESS",
            "WAITING",
            "NEUTRAL",
            "UNKNOWN",
        ):
            self.configure(checks=f"pass\t{state}")
            self.assertNotEqual(self.merge().returncode, 0, state)
        self.assert_not_called("git", "tag")
        self.assertFalse(
            any(call[:3] == ["gh", "pr", "merge"] for call in self.state()["calls"])
        )

    def test_merge_refuses_nonpassing_buckets_and_no_ci(self):
        for checks in (
            "",
            "pending\tSUCCESS",
            "skipping\tSKIPPED",
            "fail\tFAILURE",
            "cancel\tCANCELLED",
        ):
            self.configure(checks=checks)
            self.assertNotEqual(self.merge().returncode, 0)
        self.assert_not_called("git", "tag")

    def test_merge_refuses_checks_command_failure(self):
        self.configure(checks_error=8)
        self.assertNotEqual(self.merge().returncode, 0)
        self.assert_not_called("git", "tag")

    def test_merge_refuses_closed_wrong_base_and_protected_head(self):
        for settings in (
            {"pr_state": "CLOSED"},
            {"pr_state": "OPEN", "base": "develop"},
            {"base": "main", "head_branch": "main"},
            {"head_branch": "master"},
        ):
            self.configure(**settings)
            self.assertNotEqual(self.merge().returncode, 0)
        self.assert_not_called("git", "tag")

    def test_merge_refuses_tag_collisions_and_remote_error(self):
        for settings in (
            {"local_ref": True},
            {"local_ref": False, "remote_code": 0},
            {"remote_code": 128},
        ):
            self.configure(**settings)
            self.assertNotEqual(self.merge().returncode, 0)
        self.assert_not_called("git", "tag")

    def test_merge_refuses_unsafe_tags(self):
        for tag in (
            "",
            "-bad",
            "bad tag",
            "a..b",
            "a@{b",
            "a\\b",
            "a//b",
            "a/",
            "bad:tag",
        ):
            self.assertNotEqual(self.merge(tag=tag).returncode, 0)
        self.assert_not_called("git", "tag")

    def test_merge_confirmation_and_head_change(self):
        self.assertNotEqual(self.merge(confirmation="yes\n").returncode, 0)
        self.configure(head_changed=True, pr_reads=0)
        self.assertNotEqual(self.merge().returncode, 0)
        self.assertFalse(
            any(call[:3] == ["gh", "pr", "merge"] for call in self.state()["calls"])
        )

    def test_merge_queue_does_not_create_tag(self):
        self.configure(queued_merge=True)
        self.assertNotEqual(self.merge().returncode, 0)
        self.assert_not_called("git", "tag")
        self.assert_not_called("git", "push")

    def test_merge_rechecks_ci_after_confirmation(self):
        self.configure(checks_changed=True)
        self.assertNotEqual(self.merge().returncode, 0)
        self.assertFalse(
            any(call[:3] == ["gh", "pr", "merge"] for call in self.state()["calls"])
        )
        self.assert_not_called("git", "tag")

    def reset_mock(self, **settings):
        state = {"branch": "feature/example", "staged": [], "calls": [], "dirty": False}
        state.update(settings)
        self.state_path.write_text(json.dumps(state))

    def milestone_merge(self, confirmation="merge PR #12\n", args=None):
        return self.run_script(
            "merge-milestone",
            args if args is not None else ["--pr", "12"],
            confirmation,
        )

    def finish_args(self):
        return [
            "--message",
            "Milestone",
            "--title",
            "PR title",
            "--body",
            "PR body",
            "--files",
            "approved.txt",
        ]

    def finish(
        self,
        args=None,
        confirmation='commit "Milestone" with approved files\nmerge PR #12\n',
    ):
        return self.run_script(
            "finish-milestone",
            self.finish_args() if args is None else args,
            confirmation,
        )

    def assert_no_merge(self):
        self.assertFalse(
            any(call[:3] == ["gh", "pr", "merge"] for call in self.state()["calls"])
        )

    def assert_no_publication(self):
        for call in self.state()["calls"]:
            self.assertNotIn(
                call[:2],
                (
                    ["git", "tag"],
                    ["git", "reset"],
                    ["gh", "release"],
                    ["gh", "deployment"],
                ),
            )
            self.assertNotIn("--force", call)
            self.assertNotIn("-f", call)
            self.assertFalse(any("refs/tags/" in arg for arg in call))
            self.assertNotEqual(call[:3], ["git", "add", "-A"])

    def assert_failed(self, result):
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertNotIn("[PASS] PR #12 merged.", result.stdout)
        self.assertNotIn("[PASS] Milestone finished", result.stdout)
        self.assert_no_publication()

    def test_milestone_merge_success_and_sync(self):
        result = self.milestone_merge()
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.state()["calls"]
        merge_call = [
            "gh",
            "pr",
            "merge",
            "12",
            "--squash",
            "--delete-branch",
            "--match-head-commit",
            HEAD,
        ]
        self.assertIn(merge_call, calls)
        self.assertIn(["git", "switch", "main"], calls)
        self.assertIn(["git", "pull", "--ff-only", "origin", "main"], calls)
        self.assertIn(
            ["git", "rev-parse", "--verify", "refs/remotes/origin/main"], calls
        )
        self.assertIn(["git", "merge-base", "--is-ancestor", MERGE, "main"], calls)
        self.assertIn(["git", "fetch", "--prune"], calls)
        self.assertIn(["git", "log", "--oneline", "--decorate", "-8"], calls)
        self.assertEqual(self.state()["branch"], "main")
        self.assertEqual(self.state()["check_reads"], 2)
        self.assertIn("PR #12", result.stdout)
        self.assertIn("Example milestone", result.stdout)
        self.assertIn(HEAD, result.stdout)
        self.assertIn(URL, result.stdout)
        self.assertIn(MERGE, result.stdout)
        self.assertIn("Current branch: main. Working tree is clean.", result.stdout)
        self.assert_no_publication()
        self.assert_not_called("git", "push")

    def test_milestone_merge_rejects_malformed_arguments(self):
        for args in (
            [],
            ["--pr"],
            ["--pr", ""],
            ["--pr", "0"],
            ["--pr", "-1"],
            ["--pr", "1.2"],
            ["--pr", "abc"],
            ["--pr", "01"],
            ["--pr", "12", "--yes"],
            ["--pr", "12", "--force"],
            ["--pr", "12", "--pr", "12"],
        ):
            with self.subTest(args=args):
                self.reset_mock()
                self.assert_failed(self.milestone_merge(args=args))
                self.assertEqual(self.state()["calls"], [])

    def test_milestone_merge_preflight_failures(self):
        for settings in (
            {"dirty": True},
            {"no_repo": True},
            {"auth_error": 1},
            {"view_error": True},
        ):
            with self.subTest(settings=settings):
                self.reset_mock(**settings)
                self.assert_failed(self.milestone_merge())
                self.assert_no_merge()

    def test_milestone_merge_rejects_invalid_pr_metadata(self):
        for settings in (
            {"pr_state": "CLOSED"},
            {"pr_state": "MERGED"},
            {"base": "develop"},
            {"base": ""},
            {"head_branch": "main"},
            {"head_branch": "master"},
            {"head_branch": ""},
            {"head_branch": "-unsafe"},
            {"head_branch": "a..b"},
            {"head_oid": ""},
            {"head_oid": "null"},
            {"head_oid": "a" * 39},
            {"head_oid": "a" * 41},
            {"head_oid": "z" * 40},
            {"pr_number": "13"},
        ):
            with self.subTest(settings=settings):
                self.reset_mock(**settings)
                self.assert_failed(self.milestone_merge())
                self.assert_no_merge()

    def test_milestone_merge_rejects_every_unsuccessful_ci_state(self):
        states = (
            "PENDING",
            "QUEUED",
            "IN_PROGRESS",
            "WAITING",
            "NEUTRAL",
            "SKIPPED",
            "CANCELLED",
            "FAILURE",
            "ERROR",
            "UNKNOWN",
            "",
            "COMPLETED",
        )
        for state in states:
            with self.subTest(state=state):
                self.reset_mock(checks=f"pass\t{state}")
                self.assert_failed(self.milestone_merge())
                self.assert_no_merge()
        for checks in (
            "",
            "pending\tSUCCESS",
            "cancel\tSUCCESS",
            "fail\tSUCCESS",
            "unknown\tSUCCESS",
            "pass\tSUCCESS\nfail\tFAILURE",
            "pass\tSUCCESS\npass\tSKIPPED",
        ):
            with self.subTest(checks=checks):
                self.reset_mock(checks=checks)
                self.assert_failed(self.milestone_merge())
                self.assert_no_merge()

    def test_milestone_merge_ci_api_errors(self):
        for settings in (
            {"checks_error": 8},
            {"checks_json_error": True},
            {"check_count": 0},
        ):
            with self.subTest(settings=settings):
                self.reset_mock(**settings)
                self.assert_failed(self.milestone_merge())
                self.assert_no_merge()

    def test_milestone_merge_requires_exact_confirmation(self):
        for confirmation in (
            "",
            "yes\n",
            "merge PR #13\n",
            "merge PR #12 \n",
            "merge PR #12 and tag v1\n",
        ):
            with self.subTest(confirmation=confirmation):
                self.reset_mock()
                self.assert_failed(self.milestone_merge(confirmation=confirmation))
                self.assert_no_merge()

    def test_milestone_merge_revalidates_after_confirmation(self):
        for settings in (
            {"head_changed": True},
            {"branch_changed": True},
            {"checks_changed": True},
            {"checks_state_changed": True},
            {"dirty_on_read": 2},
            {"dirty_on_read": 3},
            {"pr_updates": {"2": {"head_oid": "c" * 40}}},
            {"pr_updates": {"3": {"pr_state": "CLOSED"}}},
            {"pr_updates": {"3": {"base": "develop"}}},
            {"pr_updates": {"3": {"view_error": True}}},
            {"pr_updates": {"4": {"head_oid": "c" * 40}}},
        ):
            with self.subTest(settings=settings):
                self.reset_mock(**settings)
                self.assert_failed(self.milestone_merge())
                self.assert_no_merge()
        self.reset_mock(checks_state_changed=True)
        self.assert_failed(self.milestone_merge())
        self.assertEqual(self.state()["check_reads"], 2)

    def test_milestone_merge_rejects_queue_and_invalid_merge_sha(self):
        for settings in (
            {"queued_merge": True},
            {"merge_oid": ""},
            {"merge_oid": "invalid"},
            {"merge_oid": "b" * 41},
        ):
            with self.subTest(settings=settings):
                self.reset_mock(**settings)
                self.assert_failed(self.milestone_merge())
                self.assert_not_called("git", "switch")
                self.assert_not_called("git", "pull")

    def test_milestone_merge_rejects_failed_merge_or_post_merge_query(self):
        for call in (
            [
                "gh",
                "pr",
                "merge",
                "12",
                "--squash",
                "--delete-branch",
                "--match-head-commit",
                HEAD,
            ],
            [
                "gh",
                "pr",
                "view",
                "12",
                "--json",
                "state,mergeCommit",
                "--jq",
                '[.state, (.mergeCommit.oid // "")] | @tsv',
            ],
        ):
            with self.subTest(call=call):
                self.reset_mock(fail_calls=[call])
                self.assert_failed(self.milestone_merge())
                self.assert_not_called("git", "switch")

    def test_milestone_merge_main_sync_failures(self):
        for settings in (
            {"origin_main": "c" * 40},
            {"ancestor_error": 1},
            {"dirty_on_read": 4},
            {"dirty_on_read": 5},
            {"origin_after_fetch": "c" * 40},
            {"fail_calls": [["git", "pull", "--ff-only", "origin", "main"]]},
            {"fail_calls": [["git", "fetch", "--prune"]]},
            {"fail_calls": [["git", "switch", "main"]]},
            {
                "fail_calls": [
                    ["git", "rev-parse", "--verify", "refs/remotes/origin/main"]
                ]
            },
        ):
            with self.subTest(settings=settings):
                self.reset_mock(**settings)
                self.assert_failed(self.milestone_merge())
                self.assertTrue(self.state()["merged"])
        self.reset_mock(origin_main="c" * 40)
        self.assertIn(
            "local main differs from origin/main", self.milestone_merge().stderr
        )

    def test_finish_happy_path_delegates_in_order(self):
        self.configure(dirty=True)
        result = self.finish()
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.state()["calls"]
        commit = ["git", "commit", "-m", "Milestone"]
        push = ["git", "push", "-u", "origin", "feature/example"]
        create = [
            "gh",
            "pr",
            "create",
            "--base",
            "main",
            "--head",
            "feature/example",
            "--title",
            "PR title",
            "--body",
            "PR body",
        ]
        watch = ["gh", "pr", "checks", URL, "--watch"]
        merge = [
            "gh",
            "pr",
            "merge",
            "12",
            "--squash",
            "--delete-branch",
            "--match-head-commit",
            HEAD,
        ]
        indices = [calls.index(call) for call in (commit, push, create, watch, merge)]
        self.assertEqual(indices, sorted(indices))
        self.assertIn(
            'Type exactly: commit "Milestone" with approved files', result.stdout
        )
        self.assertIn("Type exactly: merge PR #12", result.stdout)
        self.assertIn("Resolved PR #12", result.stdout)
        self.assertEqual(self.state()["branch"], "main")
        self.assertFalse(self.state()["dirty"])
        self.assertTrue(self.state()["merged"])
        self.assert_no_publication()

    def test_finish_preserves_explicit_files_and_literal_arguments(self):
        files = [
            "space name.txt",
            "line\nbreak.txt",
            "literal*[x].txt",
            "-option.txt",
            "$(touch INJECTED).txt",
        ]
        for file in files:
            (self.root / file).write_text("change\n")
        message = 'Message "quoted" $(touch INJECTED) `touch INJECTED` ; spaces'
        title = "Title with spaces\nand $(touch INJECTED)"
        body = "Body 'quoted' \"double\"\n$(touch INJECTED) `touch INJECTED` ;\n\n"
        args = [
            "--message",
            message,
            "--title",
            title,
            "--body",
            body,
            "--files",
            *files,
        ]
        result = self.finish(
            args, f'commit "{message}" with approved files\nmerge PR #12\n'
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.state()["calls"]
        self.assertEqual(
            [call for call in calls if call[:2] == ["git", "add"]],
            [["git", "add", "--", file] for file in files],
        )
        self.assertIn(["git", "commit", "-m", message], calls)
        create = next(call for call in calls if call[:3] == ["gh", "pr", "create"])
        self.assertEqual(create[create.index("--title") + 1], title)
        self.assertEqual(create[create.index("--body") + 1], body)
        self.assertFalse((self.root / "INJECTED").exists())
        self.assert_no_publication()

    def test_finish_body_file_preserves_trailing_newlines(self):
        body = "File body\nwith 'quotes' and $(touch INJECTED)\n\n"
        path = self.root / "body with spaces.txt"
        path.write_text(body)
        args = self.finish_args()
        args[4:6] = ["--body-file", str(path)]
        result = self.finish(args)
        self.assertEqual(result.returncode, 0, result.stderr)
        create = next(
            call for call in self.state()["calls"] if call[:3] == ["gh", "pr", "create"]
        )
        self.assertEqual(create[create.index("--body") + 1], body)
        self.assertFalse((self.root / "INJECTED").exists())

    def test_finish_rejects_bad_arguments_before_commit(self):
        prefix = ["--message", "Milestone", "--title", "PR title"]
        tail = ["--files", "approved.txt"]
        cases = (
            [],
            prefix + tail,
            prefix + ["--body", "Body", "--body-file", "missing"] + tail,
            prefix + ["--body", "Body", "--body", "Again"] + tail,
            prefix + ["--message", "Again", "--body", "Body"] + tail,
            prefix + ["--title", "Again", "--body", "Body"] + tail,
            prefix + ["--body-file", "missing"] + tail,
            prefix + ["--body-file", "missing", "--body-file", "missing"] + tail,
            prefix + ["--body", ""] + tail,
            prefix + ["--body", " "] + tail,
            ["--message", "", "--title", "Title", "--body", "Body"] + tail,
            ["--message", "Milestone", "--title", "", "--body", "Body"] + tail,
            ["--message", "two\nlines", "--title", "Title", "--body", "Body"] + tail,
            prefix + ["--body", "Body"],
            prefix + ["--body", "Body", "--files"],
            prefix + ["--body", "--files", "approved.txt"],
            ["--message"],
            ["--title"],
            ["--body"],
            ["--body-file"],
            prefix + ["--body", "Body"] + tail + ["--yes"],
            prefix + ["--body", "Body"] + tail + ["--files", "approved.txt"],
        )
        for args in cases:
            with self.subTest(args=args):
                self.reset_mock()
                self.assert_failed(self.finish(args))
                self.assert_not_called("git", "commit")
                self.assert_not_called("git", "push")
                self.assert_no_merge()

    def test_finish_empty_body_file_rejected(self):
        path = self.root / "empty-body.txt"
        path.write_text("\n \n")
        args = self.finish_args()
        args[4:6] = ["--body-file", str(path)]
        self.assert_failed(self.finish(args))
        self.assert_not_called("git", "commit")

    def test_finish_commit_cancel_or_failure_stops_pipeline(self):
        for settings, confirmation in (
            ({}, "yes\nmerge PR #12\n"),
            (
                {"commit_error": True},
                'commit "Milestone" with approved files\nmerge PR #12\n',
            ),
            ({"add_failure": "approved.txt"}, ""),
            ({"diff_error": 2}, ""),
        ):
            with self.subTest(settings=settings):
                self.reset_mock(**settings)
                self.assert_failed(self.finish(confirmation=confirmation))
                self.assert_not_called("git", "push")
                self.assert_no_merge()
                self.assertEqual(self.state()["staged"], [])

    def test_finish_rejects_initial_unsafe_index_or_branch(self):
        for settings in (
            {"staged": ["unapproved.txt"]},
            {"inject_unapproved": True},
            {"branch": "main"},
            {"branch": "master"},
            {"detached": True},
            {"no_repo": True},
            {"auth_error": 1},
        ):
            with self.subTest(settings=settings):
                self.reset_mock(**settings)
                self.assert_failed(self.finish())
                self.assert_not_called("git", "commit")
                self.assert_not_called("git", "push")
                self.assert_no_merge()

    def test_finish_dirty_or_changed_branch_after_commit_stops_push(self):
        for settings in (
            {"dirty": True, "unapproved_dirty": True},
            {"branch_after_commit": "feature/other"},
        ):
            with self.subTest(settings=settings):
                self.reset_mock(**settings)
                self.assert_failed(self.finish())
                self.assert_not_called("git", "push")
                self.assert_no_merge()

    def test_finish_pr_or_ci_failure_prevents_merge(self):
        for settings in (
            {"create_error": 1},
            {"created_url": ""},
            {"watch_error": 1},
            {"check_count": 0},
            {"checks": "pass\tFAILURE"},
            {"checks_json_error": True},
            {"old_gh": True, "checks": "pass\tPENDING"},
            {"view_error": True},
            {"fail_calls": [["git", "push", "-u", "origin", "feature/example"]]},
        ):
            with self.subTest(settings=settings):
                self.reset_mock(**settings)
                self.assert_failed(self.finish())
                self.assert_no_merge()

    def test_finish_resolved_pr_must_match_branch_base_and_head(self):
        for settings in (
            {"head_branch": "feature/other"},
            {"base": "develop"},
            {"pr_state": "CLOSED"},
            {"head_oid": "c" * 40},
            {"head_oid": ""},
            {"pr_number": "0"},
            {"pr_number": ""},
        ):
            with self.subTest(settings=settings):
                self.reset_mock(**settings)
                self.assert_failed(self.finish())
                self.assert_no_merge()

    def test_finish_resolves_exact_starting_branch(self):
        self.configure(branch="feature/another", head_branch="feature/another")
        result = self.finish()
        self.assertEqual(result.returncode, 0, result.stderr)
        reads = [
            call for call in self.state()["calls"] if call[:3] == ["gh", "pr", "view"]
        ]
        self.assertTrue(any(call[3] == "feature/another" for call in reads))
        self.assertIn(
            ["git", "push", "-u", "origin", "feature/another"], self.state()["calls"]
        )

    def test_finish_rejects_head_or_branch_changes_during_pr_creation(self):
        for changes in ({"local_head": "c" * 40}, {"branch": "feature/other"}):
            with self.subTest(changes=changes):
                # Simulate another process changing Git state during PR creation.
                self.reset_mock(after_create=changes)
                self.assert_failed(self.finish())
                self.assert_no_merge()

    def test_finish_wrong_merge_confirmation_stops_merge(self):
        self.assert_failed(
            self.finish(confirmation='commit "Milestone" with approved files\nyes\n')
        )
        self.assertTrue(self.state()["committed"])
        self.assert_no_merge()

    def test_finish_head_change_or_ci_change_stops_merge(self):
        for settings in (
            {"head_changed": True},
            {"checks_state_changed": True},
            {"dirty_on_read": 4},
        ):
            with self.subTest(settings=settings):
                self.reset_mock(**settings)
                self.assert_failed(self.finish())
                self.assert_no_merge()

    def test_finish_post_merge_failure_does_not_claim_success(self):
        for settings in (
            {"queued_merge": True},
            {"origin_main": "c" * 40},
            {"ancestor_error": 1},
            {"dirty_on_read": 8},
        ):
            with self.subTest(settings=settings):
                self.reset_mock(**settings)
                self.assert_failed(self.finish())


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--mock":
        run_mock()
    unittest.main()
