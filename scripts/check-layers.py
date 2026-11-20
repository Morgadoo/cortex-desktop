#!/usr/bin/env python3
"""Checks that the chat engine's dependencies point down its layers.

The folders of src-tauri/src/chat/ sit in layers (below). Code may use its own layer and the
ones beneath it; a `use` or path that reaches a higher layer is a violation. Known violations
are listed in scripts/check-layers-allow.txt, one `from -> to` module pair per line, and the
list may only shrink: the check fails on a new violation, and also on a listed one that no
longer exists (delete its line).

    python3 scripts/check-layers.py            # check
    python3 scripts/check-layers.py --write    # rewrite the allow-list from the code

Exit code 0 when the code matches the allow-list.
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
CHAT = ROOT / "src-tauri" / "src" / "chat"
ALLOW = ROOT / "scripts" / "check-layers-allow.txt"

# Bottom first. A folder may use folders in its own layer or any layer below it.
LAYERS = [
    ["storage"],
    ["models", "grounding", "toolkit", "multimedia", "ui"],
    ["turn", "automation"],
    ["commands", "cli", "integrations"],
]
LEVEL = {folder: i for i, layer in enumerate(LAYERS) for folder in layer}


def module_folders():
    """Each chat module's name and the folder it lives in, from the folders' mod.rs files."""
    out = {}
    for mod_rs in CHAT.glob("*/mod.rs"):
        folder = mod_rs.parent.name
        for name in re.findall(r"^pub(?:\([^)]*\))? mod (\w+);", mod_rs.read_text(), re.M):
            out[name] = folder
    out["commands"] = "commands"
    return out


def strip(text):
    """The code without comments, strings or test modules, which do not count as dependencies."""
    text = re.sub(r"//[^\n]*", "", text)
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.S)
    text = re.sub(r'r#*"(?:.|\n)*?"#*', '""', text)
    text = re.sub(r'"(?:\\.|[^"\\])*"', '""', text)
    cut = re.search(r"#\[cfg\(test\)\]\s*mod \w+ \{", text)
    return text[: cut.start()] if cut else text


def edges(folders):
    """(from module, to module) for every reference from one chat module to another."""
    found = set()
    for path in sorted(CHAT.rglob("*.rs")):
        rel = path.relative_to(CHAT).parts
        if len(rel) < 2:
            continue  # chat/commands.rs and chat/state.rs are checked as commands and storage
        folder = rel[0]
        module = rel[1].removesuffix(".rs") if rel[1] != "mod.rs" else folder
        code = strip(path.read_text())
        targets = set(re.findall(r"crate::chat::(\w+)", code))
        # `use crate::chat::{a, b::c}` groups
        for group in re.findall(r"crate::chat::\{([^}]*)\}", code):
            targets |= set(re.findall(r"(?:^|,)\s*(\w+)", group))
        for target in targets:
            if target in folders and folders[target] != folder:
                found.add((f"{folder}/{module}", f"{folders[target]}/{target}"))
    return found


def violations(found):
    return sorted(
        (a, b) for a, b in found if LEVEL[b.split("/")[0]] > LEVEL[a.split("/")[0]]
    )


def main():
    folders = module_folders()
    unknown = sorted(set(folders.values()) - set(LEVEL))
    if unknown:
        print(f"check-layers: folders with no layer (add them to LAYERS): {unknown}")
        return 1
    current = [f"{a} -> {b}" for a, b in violations(edges(folders))]
    if "--write" in sys.argv:
        ALLOW.write_text(
            "# Known upward dependencies in the chat engine; see scripts/check-layers.py.\n"
            "# Remove a line when its dependency is gone. Never add one.\n"
            + "".join(line + "\n" for line in current)
        )
        print(f"check-layers: wrote {len(current)} known violations")
        return 0
    allowed = [
        line.strip()
        for line in ALLOW.read_text().splitlines()
        if line.strip() and not line.startswith("#")
    ]
    new = sorted(set(current) - set(allowed))
    gone = sorted(set(allowed) - set(current))
    for line in new:
        print(f"check-layers: new upward dependency: {line}")
    for line in gone:
        print(f"check-layers: fixed, remove from {ALLOW.name}: {line}")
    if new or gone:
        return 1
    print(f"check-layers: ok ({len(current)} known upward dependencies left)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
