#!/usr/bin/env python3
"""Checks that docs/agent-kernel-plan.md tells the truth about its tests.

Every test the plan's "Checked against the kernel document" tables and its table of the example's open
points cite must exist, and must run and pass. The tests are run by name across the Rust workspace.

    python3 scripts/check-kernel-plan.py          # from the repository root

Exit code 0 when every cited test passed, 1 otherwise.
"""

import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
PLAN = ROOT / "docs" / "agent-kernel-plan.md"
CRATE = ROOT / "src-tauri"


# The parts of the plan whose tables cite tests: (from heading, to heading).
SECTIONS = [
    ("## Checked against the kernel document", "### The other sections"),
    ("## 7. Notes on the kernel example", "Points from the review of the example"),
]


def cited_names(plan: str) -> list[tuple[str, bool]]:
    """(name, is_prefix) for each test named in the last column of the verification tables."""
    rows = []
    for first, last in SECTIONS:
        start = plan.index(first)
        rows += plan[start : plan.index(last, start)].splitlines()
    names = []
    for row in rows:
        if not row.startswith("|"):
            continue
        last = row.strip("|").split("|")[-1]
        names += [(n, bool(e)) for n, e in re.findall(r"`([a-z][a-z0-9_]+)(…)?`", last)]
    return names


def test_functions() -> set[str]:
    found = set()
    pattern = re.compile(
        r"#\[(?:tokio::)?test[^\]]*\]\s*(?:#\[[^\]]*\]\s*)*(?:async )?fn (\w+)"
    )
    for path in CRATE.rglob("*.rs"):
        if "target" in path.parts:
            continue
        found.update(pattern.findall(path.read_text(encoding="utf-8")))
    return found


def main() -> int:
    tests = test_functions()
    wanted = set()
    problems = []
    for name, prefix in cited_names(PLAN.read_text(encoding="utf-8")):
        matches = [t for t in tests if t.startswith(name)] if prefix else [name] * (name in tests)
        if len(matches) != 1:
            problems.append(f"`{name}` names {len(matches)} tests")
        else:
            wanted.add(matches[0])
    if problems:
        print("\n".join(problems))
        return 1
    run = subprocess.run(
        ["cargo", "test", "--workspace", "--features", "cortex/test-utils", "--", *sorted(wanted)],
        cwd=CRATE,
        capture_output=True,
        text=True,
    )
    results: dict[str, list[str]] = {}
    for m in re.finditer(r"^test (\S+) \.\.\. (ok|FAILED|ignored)", run.stdout, re.M):
        results.setdefault(m.group(1).split("::")[-1], []).append(m.group(2))
    missing = sorted(n for n in wanted if n not in results)
    failed = sorted(n for n in wanted if any(r != "ok" for r in results.get(n, [])))
    passed = len(wanted) - len(missing) - len(failed)
    print(f"check-kernel-plan: {passed} of {len(wanted)} cited tests passed")
    for n in missing:
        print(f"  did not run: {n}")
    for n in failed:
        print(f"  not ok: {n}")
    return 0 if not missing and not failed else 1


if __name__ == "__main__":
    sys.exit(main())
