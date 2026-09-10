"""A fixed, local sample workflow using only a newly allocated temporary workspace."""

import tempfile
from dataclasses import dataclass
from io import StringIO
from pathlib import Path

import yaml

from ghost_cli.config import initialize_home
from ghost_cli.context_pack import create_context_pack
from ghost_cli.doctor import HealthReport, inspect_health
from ghost_cli.handoffs import create_handoff
from ghost_cli.next_steps import create_next_summary
from ghost_cli.output_models import OutputType
from ghost_cli.outputs import add_output, reject_environment_path
from ghost_cli.paths import GhostError, atomic_write, create_file_if_missing, home_writer
from ghost_cli.redaction import redact_text
from ghost_cli.registry import add_project
from ghost_cli.sessions import add_note, start_session
from ghost_cli.update_packs import create_update_pack

ALIAS = "nexora-demo"
NOTICE = "DEMO / SAMPLE ONLY — fictional NEXORA-style project, not real project evidence."
SAMPLE_OUTPUT = f"""# DEMO / SAMPLE Codex/Astra output

{NOTICE}
Proposed change: display a saved price-alert threshold beside a sample product.
Suggested cases: no saved threshold, a threshold above price, and a matching price.
No implementation, model call, or test run occurred. Results remain unverified.

Fake credential examples supplied in memory to demonstrate output redaction:
api_key=demo-only-fake-api-value
Authorization: Bearer demo-only-fake-bearer-value

Next manual review: agree on acceptance criteria before any implementation.
"""


@dataclass(frozen=True)
class DemoResult:
    root: Path
    artifacts: dict[str, Path]
    health: HealthReport

    @property
    def home(self) -> Path:
        return self.root / "ghost-home"

    @property
    def report_path(self) -> Path:
        return self.root / "DEMO-REPORT.md"


def allocate_demo() -> Path:
    """Never select an existing project, even when the temp location is overridden."""
    parent = Path(tempfile.gettempdir())
    reject_environment_path(parent)
    parent = parent.resolve(strict=True)
    reject_environment_path(parent)
    # Metadata checks only: .git may be a directory or a worktree pointer file.
    for ancestor in (parent, *parent.parents):
        markers = (ancestor / ".git", ancestor / ".ghost" / "project.yaml")
        if ancestor.name == ".ghost" or any(
            marker.exists() or marker.is_symlink() for marker in markers
        ):
            raise GhostError("Demo temporary storage must be outside project repositories.")
    return Path(tempfile.mkdtemp(prefix="ghost-demo-nexora-", dir=parent))


def seed_workspace(workspace: Path, home: Path) -> None:
    """Replace only the initial templates in this run's newly created workspace."""
    with home_writer(home):
        atomic_write(
            workspace / "status.md",
            f"# DEMO / SAMPLE status\n\n{NOTICE}\n\n"
            "Scenario: a student price-comparison app with saved products and price alerts.\n"
            "Sample focus: make a saved alert threshold visible beside a product.\n"
            "Current state: design proposal only; no source code or validation evidence.\n",
        )
        atomic_write(
            workspace / "decisions.md",
            f"# DEMO / SAMPLE decisions\n\n{NOTICE}\n\n"
            "- Use fictional products and prices only.\n"
            "- Keep alert evaluation separate from presentation.\n"
            "- Ask the owner to review empty, above-price, and matching-price cases.\n"
            "- Do not treat sample progress as a real release or portfolio accomplishment.\n",
        )
        atomic_write(
            workspace / "milestones.yaml",
            yaml.safe_dump(
                {
                    "version": 1,
                    "milestones": [
                        {
                            "name": "DEMO / SAMPLE: price-alert threshold display",
                            "status": "proposed",
                            "acceptance_criteria": [
                                "Review the three fictional alert cases before implementation.",
                                "Supply actual validation evidence; none is claimed here.",
                            ],
                        }
                    ],
                },
                sort_keys=False,
            ),
        )


def build_demo(root: Path) -> DemoResult:
    """Coordinate existing modules with an explicit home; never change the environment."""
    home = initialize_home(root / "ghost-home")
    project_path = root / ALIAS
    project_path.mkdir(mode=0o700)
    project = add_project(ALIAS, project_path, "NEXORA — DEMO / SAMPLE ONLY", home=home)
    workspace = project.path / ".ghost"
    seed_workspace(workspace, home)
    session = start_session(
        ALIAS, "DEMO / SAMPLE: review a fictional price-alert display proposal.", home=home
    )
    add_note(
        "DEMO / SAMPLE: acceptance cases drafted; no code changed or tests executed. "
        "Owner review is still required.",
        ALIAS,
        home=home,
    )
    artifacts = {
        "Project workspace": workspace,
        "Session record": workspace / "sessions" / session.id / "session.yaml",
        "Session note": workspace / "sessions" / session.id / "notes.md",
        "Sanitized sample output": add_output(
            OutputType.codex,
            ALIAS,
            title="DEMO / SAMPLE Codex/Astra proposal",
            stdin=StringIO(SAMPLE_OUTPUT),
            home=home,
        ),
        "Context pack": create_context_pack(ALIAS, home=home),
    }
    for tool in ("codex", "chatgpt", "gemini", "antigravity"):
        artifacts[f"{tool} handoff"] = create_handoff(ALIAS, tool, home=home)
    artifacts["Next-step draft"] = create_next_summary(ALIAS, home=home)
    pack = create_update_pack(ALIAS, home=home)
    for path in sorted(pack.iterdir()):
        artifacts[f"Update pack: {path.name}"] = path
    return DemoResult(root, artifacts, inspect_health(home=home))


def render_demo_report(result: DemoResult) -> str:
    counts = {
        level: sum(finding.level == level for finding in result.health.findings)
        for level in ("PASS", "WARN", "ERROR")
    }
    lines = [
        "# GHOST NEXORA demo report",
        "",
        NOTICE,
        "",
        f"Demo directory: {result.root}",
        f"Isolated GHOST_HOME for manual follow-up: {result.home}",
        f"Registered alias: {ALIAS}",
        "",
        "## Created artifacts (paths relative to the demo directory)",
        "",
        *(
            f"- {label}: {path.relative_to(result.root)}"
            for label, path in result.artifacts.items()
        ),
        "",
        "## Doctor — actual local storage checks",
        "",
        " / ".join(f"{level} ({count})" for level, count in counts.items()),
        *(
            f"- {item.level}: {item.scope} / {item.check}: {item.message}"
            for item in result.health.findings
        ),
        "",
        "Storage health does not verify the sample claims, tests, or release readiness.",
        "",
        "## Next manual review steps",
        "",
        "1. Review the sample status, decisions, milestones, session note, and sanitized output.",
        "2. Inspect the context pack, four handoffs, next-step draft, and six update drafts.",
        "3. Keep all DEMO / SAMPLE labels; these drafts are not real project evidence.",
        "4. Supply owner-approved scope and actual validation evidence before any real work.",
        "5. The demo session remains active. Select the isolated home above before using",
        f"   ghost session status {ALIAS} or manually closing it with ghost session close {ALIAS}.",
        "",
        "Artifacts are retained for review. A new run creates a separate temporary directory.",
        "The OS may clean temporary storage; copy reviewed sample artifacts if needed.",
        "No AI calls, shell execution, GitHub automation, or publication occurred.",
    ]
    return redact_text("\n".join(lines)) + "\n"


def create_nexora_demo() -> DemoResult:
    root = allocate_demo()
    try:
        create_file_if_missing(
            root / "DEMO-REPORT.md",
            f"# Incomplete GHOST demo\n\n{NOTICE}\n\n"
            "This run has not finished. Preserve partial artifacts for inspection.\n"
            "Run ghost demo nexora again for a separate, fresh demo.\n",
        )
        result = build_demo(root)
        atomic_write(result.report_path, render_demo_report(result))
    except (GhostError, OSError, RuntimeError):
        # Do not echo arbitrary storage errors or remove evidence from an interrupted run.
        raise GhostError(
            f"Demo incomplete. Partial artifacts preserved at {redact_text(str(root))}. "
            "Check storage permissions and inspect this directory; rerun for a fresh demo."
        ) from None
    return result
