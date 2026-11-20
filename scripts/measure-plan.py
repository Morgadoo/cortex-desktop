#!/usr/bin/env python3
"""Prints the measures of docs/codebase-improvements-plan.md section 7, from the code as it is.

    python3 scripts/measure-plan.py            # every measure, with its target
    python3 scripts/measure-plan.py --check    # and fail when one misses its target

Rust measures cover the app (`src-tauri/src`) and the workspace's crates, without test code
(test files, test modules) and without generated files (`src/handlers.rs`). The interface
tests and the checks that have scripts of their own (layers, statics) are run as they are.
"""

import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
RUST = [ROOT / "src-tauri" / "src", ROOT / "src-tauri" / "crates"]
GENERATED = {"src-tauri/src/handlers.rs"}


def rel(path):
    return path.relative_to(ROOT).as_posix()


def rust_files():
    for base in RUST:
        for path in sorted(base.rglob("*.rs")):
            r = rel(path)
            if "/target/" in r or "/wasm/" in r or r in GENERATED:
                continue
            if path.name in ("tests.rs", "testing.rs") or path.stem.endswith("_tests"):
                continue
            yield path


def without_tests(lines):
    """The lines before a test module (`#[cfg(test)]` then `mod`), if there is one."""
    for i, line in enumerate(lines):
        if line.strip() == "#[cfg(test)]" and i + 1 < len(lines):
            if re.match(r"\s*(pub(\([^)]*\))?\s+)?mod \w+", lines[i + 1]):
                return lines[:i]
    return lines


FN = re.compile(r"^(\s*)(pub(\([^)]*\))?\s+)?(const\s+)?(async\s+)?(unsafe\s+)?fn\s+(\w+)")


def long_functions(limit):
    found = []
    for path in rust_files():
        lines = without_tests(path.read_text(encoding="utf-8").split("\n"))
        for i, line in enumerate(lines):
            m = FN.match(line)
            if not m or line.rstrip().endswith(";"):
                continue
            indent = len(m.group(1))
            end = i
            # The body ends at the first `}` back at the signature's indent.
            for j in range(i + 1, len(lines)):
                if lines[j].rstrip() == " " * indent + "}":
                    end = j
                    break
                if lines[j].strip() and not lines[j].startswith(" " * (indent + 1)) and j > i + 1:
                    if not lines[j].startswith(" " * indent + ")") and not lines[j].startswith(" " * indent + "where"):
                        break
            length = end - i + 1
            if length > limit:
                found.append((length, f"{rel(path)}:{i + 1} {m.group(7)}"))
    return sorted(found, reverse=True)


def long_files(limit):
    found = []
    for path in rust_files():
        n = len(without_tests(path.read_text(encoding="utf-8").split("\n")))
        if n > limit:
            found.append((n, rel(path)))
    return sorted(found, reverse=True)


def run(cmd, cwd=ROOT):
    return subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, shell=isinstance(cmd, str))


def grep_count(pattern, *paths, extra=None):
    out = run(["grep", "-rnE", pattern, *paths]).stdout.splitlines()
    if extra:
        out = [l for l in out if extra(l)]
    return len(out)


def main():
    rows = []

    def row(name, value, ok, detail=""):
        rows.append((name, value, ok, detail))

    ignored = run("git ls-files -ci --exclude-standard").stdout.split()
    row("Tracked files that are ignored", len(ignored), not ignored)

    test = run("npm test")
    fails = sum(int(n) for n in re.findall(r"^# fail (\d+)", test.stdout, re.M))
    row("Interface tests failing", fails, test.returncode == 0 and fails == 0)

    toml = (ROOT / "src-tauri/permissions/commands.toml").read_text()
    sizes = [
        (len(re.findall(r'^\s*"[a-z_0-9]+",?\s*$', block, re.M)), name)
        for name, block in re.findall(r'identifier\s*=\s*"([^"]+)"(.*?)(?=\[\[permission\]\]|\Z)', toml, re.S)
    ]
    biggest = max(sizes) if sizes else (0, "")
    row("Commands in the largest permission", biggest[0], biggest[0] < 50, biggest[1])

    loaders = grep_count(r"and_then\(.*from_str", "src-tauri/src", extra=lambda l: "unwrap_or_default" in l)
    row("Hand-written settings loaders", loaders, loaders == 0)

    clients = run(["grep", "-rn", r"reqwest::Client::\(builder\|new\)", "src-tauri/src"]).stdout.splitlines()
    row("HTTP clients built in place", len(clients), len(clients) <= 10, "each for a reason, in the plan's departures")

    toasts = run(["grep", "-rn", "toast(String(e)", "webview"]).stdout.splitlines()
    row("Raw error toasts", len(toasts), len(toasts) <= 1, "the helper itself")

    chat = [p for p in (ROOT / "src-tauri/src/chat").iterdir() if p.is_file()]
    row("Files directly in src-tauri/src/chat/", len(chat), len(chat) < 15)

    app = [p for p in (ROOT / "webview/pages/app").iterdir() if p.is_file()]
    row("Files directly in webview/pages/app/", len(app), len(app) < 5)

    ts = [
        (len(p.read_text(encoding="utf-8").split("\n")), rel(p))
        for p in (ROOT / "webview").rglob("*.ts")
        if "generated" not in p.parts and "node_modules" not in p.parts
    ]
    largest = max(ts)
    row("Largest interface file", largest[0], largest[0] < 800, largest[1])

    fns = long_functions(150)
    row("Rust functions over 150 lines", len(fns), not fns, "; ".join(f"{n} {w}" for n, w in fns))

    files = long_files(800)
    row("Rust files over 800 lines before their test module", len(files), not files, "; ".join(f"{n} {w}" for n, w in files))

    allow = [l for l in (ROOT / "scripts/check-layers-allow.txt").read_text().splitlines() if l.strip() and not l.startswith("#")]
    layers = run(["python3", "scripts/check-layers.py"])
    row("Layering violations allowed", len(allow), not allow and layers.returncode == 0)

    public = grep_count(r"^\s*pub (async )?fn", "src-tauri/src/chat")
    row("Public functions in chat", public, public < 400)

    strings = run(["grep", "-rnE", r"Result<[^;{]*, String>", "src-tauri/src/chat", "--include=*.rs"]).stdout.splitlines()
    below = [
        l for l in strings
        if not re.search(r"src/chat/(commands|cli|integrations)/|src/chat/commands\.rs|tests\.rs|_tests\.rs", l)
        and "engine/steps.rs" not in l  # the kernel's Steps trait takes text
    ]
    tested = [l for l in below if "automation/org.rs" in l]  # a test helper in a test module
    below = [l for l in below if l not in tested]
    row("Result<_, String> in chat below the entry points", len(below), not below, "the kernel's two Steps methods keep its type")

    statics = run(["python3", "scripts/check-statics.py"])
    count = re.search(r"(\d+) mutable statics", statics.stdout + statics.stderr)
    row("Mutable process-wide statics", int(count.group(1)) if count else -1, statics.returncode == 0)

    handlers = (ROOT / "src-tauri/src/handlers.rs").read_text()
    native = handlers.split('#[cfg(feature = "hosted")]')[0]
    hosted = re.findall(r"crate::(schedules|skills)::\w+", native)
    signin = re.findall(r"crate::browser_login::\w+", native)
    row(
        "Hosted-only commands in the default build",
        len(hosted) + len(signin),
        not hosted,
        f"schedule/skill: {len(hosted)}; sign-in: {len(signin)} (kept, see the plan's departures)",
    )

    generated = run(["python3", "scripts/generate-commands.py", "--check"])
    row("Hand edits to add a command", "1, plus a script", generated.returncode == 0)

    calls = [
        l for l in run(["grep", "-rn", "tauriInvoke[<(]", "webview/pages/app"]).stdout.splitlines()
        if "core/" not in l
    ]
    row("Backend calls by string name", len(calls), not calls)

    width = max(len(r[0]) for r in rows)
    for name, value, ok, detail in rows:
        print(f"{'ok  ' if ok else 'MISS'} {name:<{width}}  {value}{'  (' + detail + ')' if detail else ''}")
    if "--check" in sys.argv and not all(r[2] for r in rows):
        sys.exit(1)


if __name__ == "__main__":
    main()
