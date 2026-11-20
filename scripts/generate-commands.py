#!/usr/bin/env python3
"""Generates the IPC surface from the #[tauri::command] functions in src-tauri/src/.

Writes two files, both committed:

  src-tauri/src/handlers.rs           the generate_handler! list app.rs registers
  webview/pages/app/core/generated/commands.ts   the command names, as a TypeScript union
  src-tauri/permissions/commands.toml one permission per source file, allowing its commands

Commands that exist only in hosted builds (their module is declared behind
`#[cfg(feature = "hosted")]`, or the function is) are listed only in the hosted build's list.

A new command in an existing file is registered and permitted by running this script; a new
file's permission is then granted to its windows in capabilities/*.json, once. A command that
only a separate window may call (the dictation pill, the settings overlay) is marked with the
line `// IPC: own permission` directly above its #[tauri::command], and keeps a hand-written
permission in permissions/windows.toml instead.

    python3 scripts/generate-commands.py           # write both files
    python3 scripts/generate-commands.py --check   # fail when either is out of date (CI)
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
SRC = ROOT / "src-tauri" / "src"
HANDLERS = SRC / "handlers.rs"
COMMAND_NAMES = ROOT / "webview" / "pages" / "app" / "core" / "generated" / "commands.ts"
PERMISSIONS = ROOT / "src-tauri" / "permissions" / "commands.toml"
MARKER = "// IPC: own permission"

COMMAND = re.compile(
    r"(?P<marker>[ \t]*// IPC: own permission[^\n]*\n)?"
    r"[ \t]*#\[tauri::command[^\]]*\][^\n]*\n"
    r"(?:[ \t]*#\[[^\n]*\n|[ \t]*//[^\n]*\n)*"
    r"[ \t]*pub(?:\([^)]*\))?\s+(?:async\s+)?fn\s+(?P<name>\w+)"
)


def module_path(path):
    """The path the handler list names a command by: its module, or the re-export it has."""
    rel = path.relative_to(SRC).with_suffix("").parts
    if rel[-1] in ("mod", "lib"):
        rel = rel[:-1]
    # Each chat/commands/*_commands.rs is glob re-exported from chat::commands.
    if len(rel) == 3 and rel[:2] == ("chat", "commands") and rel[2].endswith("_commands"):
        rel = rel[:2]
    return "::".join(rel)


def summary(text):
    """The first sentence of a file's `//!` comment, for its permission's description."""
    lines = []
    for line in text.splitlines():
        if not line.startswith("//!"):
            if lines:
                break
            continue
        body = line[3:].strip()
        if not body:
            if lines:
                break
            continue
        lines.append(body)
    first = " ".join(lines)
    first = re.split(r"(?<=[.:])\s", first, maxsplit=1)[0].rstrip(":")
    first = re.sub(r"[`\[\]]", "", first).replace('"', "'")
    return first or "Commands of this file."


HOSTED_CFG = '#[cfg(feature = "hosted")]'


def hosted_files():
    """Files compiled only with the `hosted` feature: modules declared behind its cfg."""
    out = set()
    for parent in SRC.rglob("*.rs"):
        text = parent.read_text()
        for m in re.finditer(re.escape(HOSTED_CFG) + r"\s*(?:pub(?:\([^)]*\))? )?mod (\w+);", text):
            base = SRC if parent.name in ("lib.rs", "main.rs", "mod.rs") else parent.with_suffix("")
            base = parent.parent if parent.name in ("lib.rs", "main.rs", "mod.rs") else base
            name = m.group(1)
            for candidate in (base / f"{name}.rs", base / name / "mod.rs"):
                if candidate.exists():
                    out.add(candidate)
            if (base / name).is_dir():
                out |= set((base / name).rglob("*.rs"))
    return out


def scan():
    gated = hosted_files()
    files = []
    for path in sorted(SRC.rglob("*.rs")):
        if path == HANDLERS:
            continue
        text = path.read_text()
        found = []
        for m in COMMAND.finditer(text):
            # Attributes and comments directly above the command (up to a blank line).
            before = text[: m.start()].rsplit("\n\n", 1)[-1]
            hosted = path in gated or HOSTED_CFG in before
            found.append((m.group("name"), bool(m.group("marker")), hosted))
        if found:
            files.append((path, text, found))
    return files


def identifier(path):
    rel = path.relative_to(SRC).with_suffix("").parts
    return "allow-" + "-".join(p.replace("_", "-") for p in rel if p != "mod")


def handler_fn(cfg, paths):
    return (
        f"#[cfg({cfg})]\n"
        "pub(crate) fn handler(invoke: tauri::ipc::Invoke<tauri::Wry>) -> bool {\n"
        "    let commands: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool = tauri::generate_handler![\n"
        + "".join(f"        {p},\n" for p in paths)
        + "    ];\n    commands(invoke)\n}\n"
    )


def render(files):
    entries = sorted(
        (f"crate::{module_path(path)}::{name}" if module_path(path) else f"crate::{name}", hosted)
        for path, _, found in files
        for name, _, hosted in found
    )
    native = [p for p, hosted in entries if not hosted]
    everything = [p for p, _ in entries]
    handlers = (
        "//! The commands the page may call, registered in `app.rs`: the native app's, and with the\n"
        "//! `hosted` feature also the hosted web app's.\n"
        "//!\n"
        "//! Generated by `scripts/generate-commands.py` from the `#[tauri::command]` functions;\n"
        "//! do not edit. CI fails when it is out of date.\n\n"
        + handler_fn('not(feature = "hosted")', native)
        + "\n"
        + handler_fn('feature = "hosted"', everything)
    )
    blocks = [
        "# One permission per source file, allowing the commands it defines.\n"
        "# Generated by scripts/generate-commands.py; do not edit. Grant a file's permission to its\n"
        "# windows in capabilities/*.json. Commands marked `// IPC: own permission` are in\n"
        "# windows.toml instead.\n"
    ]
    for path, text, found in files:
        names = sorted(name for name, own, _ in found if not own)
        if not names:
            continue
        rel = path.relative_to(SRC.parent).as_posix()
        blocks.append(
            "[[permission]]\n"
            f'identifier = "{identifier(path)}"\n'
            f'description = "{summary(text)} ({rel})"\n'
            "commands.allow = [\n" + "".join(f'  "{n}",\n' for n in names) + "]\n"
        )
    names = sorted({name for _, _, found in files for name, _, hosted in found if not hosted})
    commands = (
        "// Generated by scripts/generate-commands.py from the #[tauri::command] functions; do not\n"
        "// edit. Every command the native app registers, so a call by a name that does not exist\n"
        "// (renamed, removed, mistyped) is a type error.\n\n"
        "export type Command =\n" + "".join(f"  | '{n}'\n" for n in names) + ";\n"
    )
    return handlers, "\n".join(blocks), commands


def main():
    handlers, permissions, commands = render(scan())
    if "--check" in sys.argv:
        stale = [
            p.relative_to(ROOT).as_posix()
            for p, want in ((HANDLERS, handlers), (PERMISSIONS, permissions), (COMMAND_NAMES, commands))
            if not p.exists() or p.read_text() != want
        ]
        if stale:
            print(f"generate-commands: out of date: {', '.join(stale)}; run python3 scripts/generate-commands.py")
            return 1
        print("generate-commands: ok")
        return 0
    HANDLERS.write_text(handlers)
    PERMISSIONS.write_text(permissions)
    COMMAND_NAMES.write_text(commands)
    print("generate-commands: wrote", HANDLERS.relative_to(ROOT), "and", PERMISSIONS.relative_to(ROOT))
    return 0


if __name__ == "__main__":
    sys.exit(main())
