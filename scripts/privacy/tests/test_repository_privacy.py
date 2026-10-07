"""Synthetic Git repositories prove index prevention and actual ignore semantics."""

from __future__ import annotations

import importlib.util
import io
import shutil
import subprocess
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location(
    "privacy_guard", ROOT / "scripts/privacy/check_repository.py"
)
assert SPEC is not None and SPEC.loader is not None
guard = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(guard)

# These paths need not exist: check-ignore tests the policy without secret files.
IGNORE_PROBES = [
    ".ghost/private.json",
    "personal-memory.json",
    "nested/.memory-lock",
    "nested/personal-memory.json",
    "nested/memory-audit.jsonl",
    "GHOST_HOME/memory/document.json",
    "nested/project/.ghost/session.json",
    ".ghost-dev/test.json",
    ".env",
    ".env.local",
    ".env.production",
    ".env.development",
    ".env.test",
    ".ENV",
    ".ENV.production",
    "secrets.log",
    "debug.log",
    "apps/cli/dist/example.whl",
    "apps/desktop/dist/index.html",
    "apps/desktop/src-tauri/target/release/example",
    ".DS_Store",
    ".venv/file",
    "nested/.venv/file",
    "nested/.env.production",
    "nested/.EnV.test",
    ".env.example",
    "nested/build/file",
    "nested/.ghost-setup-example/project.yaml",
    "ghost-home/projects.yaml",
    "nested/transcripts/example.txt",
    "nested/recordings/example.wav",
    "nested/key.pem",
    "nested/key.key",
    "nested/key.p12",
    "nested/key.pfx",
    "nested/key.mobileprovision",
    "nested/key.cer",
    "nested/audit.jsonl",
    "nested/state.sqlite",
    "nested/state.sqlite3",
    "nested/state.db",
]


class RepositoryPrivacyTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="ghost-privacy-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.git("init", "-q")
        for path in [".gitignore", "apps/desktop/.gitignore", "apps/desktop/src-tauri/.gitignore"]:
            target = self.root / path
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / path, target)

    def git(self, *args: str) -> bytes:
        return subprocess.check_output(["git", *args], cwd=self.root)

    def test_personal_memory_guard(self) -> None:
        for path in [
            "personal-memory.json",
            "nested/Personal-Memory.JSON",
            "memory-audit.jsonl",
            "GHOST_HOME/memory/document.json",
        ]:
            with self.subTest(path=path):
                self.assertIsNotNone(guard.forbidden_reason(path))
        self.assertIsNone(guard.forbidden_reason("apps/desktop/src/personal-memory.ts"))

    def test_ignore_policy(self) -> None:
        for path in IGNORE_PROBES:
            with self.subTest(path=path):
                probe = subprocess.run(
                    ["git", "check-ignore", "--no-index", "-v", path],
                    cwd=self.root, capture_output=True, check=False,
                )
                self.assertEqual(probe.returncode, 0, path)
                self.assertIn(b".gitignore:", probe.stdout)

    def test_legitimate_tracked_files_are_not_ignored_or_forbidden(self) -> None:
        for path in guard.tracked_paths(ROOT):
            with self.subTest(path=path):
                probe = subprocess.run(
                    ["git", "check-ignore", "--no-index", path],
                    cwd=self.root, capture_output=True, check=False,
                )
                self.assertEqual(probe.returncode, 1, path)
                self.assertIsNone(guard.forbidden_reason(path))

    def test_force_added_private_paths_are_rejected(self) -> None:
        paths = [
            "nested/project/.ghost/session.json", ".ENV.production", "keys/credential.pfx",
            "logs/audit.jsonl", "ghost-home/config.yaml", "recordings/sample.wav",
        ]
        for path in paths:
            target = self.root / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text("synthetic fixture\n", encoding="utf-8")
            self.git("add", "-f", "--", path)
        self.assertEqual({path for path, _ in guard.check_repository(self.root)}, set(paths))

    def test_paths_with_spaces_newlines_and_unicode_remain_separate(self) -> None:
        for path in ["source with spaces.py", "source\nnext.py", "source-\u03bb.py"]:
            (self.root / path).write_text("# synthetic\n", encoding="utf-8")
            self.git("add", "--", path)
        self.assertEqual(len(guard.tracked_paths(self.root)), 3)
        self.assertEqual(guard.check_repository(self.root), [])

    def test_case_variants_and_no_blanket_test_exemption(self) -> None:
        for path in ["tests/.ghost/state.json", "tests/.env.example", "nested/.GHOST/a.json",
                     "keys/PRIVATE.PEM", "nested/.ghost-setup-draft/a.yaml", "records/STATE.DB"]:
            with self.subTest(path=path):
                self.assertIsNotNone(guard.forbidden_reason(path))
        for path in ["tests/test_secrets.py", "source/authorization.py", "contracts/plan.json",
                     "docs/env.md", "source/env.py", "source/key.rs", "config.yaml"]:
            with self.subTest(path=path):
                self.assertIsNone(guard.forbidden_reason(path))

    def test_console_redacts_even_private_filenames(self) -> None:
        private_path = "personal-name/.ghost/private-note.json"
        output = io.StringIO()
        findings = [(private_path, "private storage")]
        with (
            patch.object(guard, "check_repository", return_value=findings),
            redirect_stdout(output),
        ):
            self.assertEqual(guard.main(), 1)
        self.assertNotIn(private_path, output.getvalue())
        self.assertNotIn("personal-name", output.getvalue())
        self.assertIn("path_sha256=", output.getvalue())


if __name__ == "__main__":
    unittest.main()
