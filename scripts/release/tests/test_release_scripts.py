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
    if tool == "git":
        command = args[0]
        if command == "rev-parse":
            if "--show-toplevel" in args:
                return (1, "") if state.get("no_repo") else (0, os.environ["MOCK_ROOT"])
            return 0, HEAD
        if command == "symbolic-ref":
            return (1, "") if state.get("detached") else (0, state["branch"])
        if command == "check-ref-format":
            if "--branch" in args and args[-1] == "HEAD":
                return 1, ""
            return (1, "") if any(c in args[-1] for c in "~^:?[**") else (0, "")
        if command == "status":
            return 0, " M approved.txt" if state.get("dirty") else ""
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
            state["committed"] = True
            state["staged"] = []
            state["dirty"] = False
            return 0, "Mock commit only"
        if command in ("push", "pull", "fetch", "tag", "merge-base", "log", "branch"):
            return 0, ""
    if tool == "gh":
        if args[0] == "auth":
            return state.get("auth_error", 0), ""
        if args[:2] == ["pr", "create"]:
            return 0, URL
        if args[:2] == ["pr", "merge"]:
            state["merged"] = True
            return 0, ""
        if args[:2] == ["pr", "view"]:
            fields = args[args.index("--json") + 1]
            if fields == "statusCheckRollup":
                return 0, str(state.get("check_count", 1))
            if fields == "state,mergeCommit":
                return 0, f"OPEN\t" if state.get("queued_merge") else f"MERGED\t{MERGE}"
            state["pr_reads"] = state.get("pr_reads", 0) + 1
            oid = (
                "c" * 40
                if state.get("head_changed") and state["pr_reads"] > 2
                else HEAD
            )
            return 0, "\t".join(
                [
                    "12",
                    state.get("pr_state", "OPEN"),
                    state.get("head_branch", "feature/example"),
                    state.get("base", "main"),
                    URL,
                    oid,
                    "Example milestone",
                ]
            )
        if args[:2] == ["pr", "checks"]:
            if "--help" in args:
                return 0, "--watch" if not state.get("old_gh") else "checks help"
            if "--watch" in args:
                return state.get("watch_error", 0), ""
            if "--json" in args:
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
                "--merge",
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


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--mock":
        run_mock()
    unittest.main()
