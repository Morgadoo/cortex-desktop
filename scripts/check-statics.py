#!/usr/bin/env python3
"""Lists the mutable process-wide statics in the Rust code and checks each says why it is one.

A mutable static is a `static` whose type can change after it is set: a lock, an atomic, a
cell, a channel, or a type or struct in the same file that holds one. A value set once and
read after (a `OnceLock` of a path, a compiled pattern) is not one. Test code (test files,
test modules, `#[cfg(test)]` functions) and the JavaScript runtime's WebAssembly guest are
left out.

Each mutable static needs a comment in the lines just above it starting `Process-wide:`, saying
why it cannot be state the app owns. The check fails over MAX of them, or on one with no reason.

    python3 scripts/check-statics.py           # check
    python3 scripts/check-statics.py --list    # and list them
"""

import pathlib
import re
import sys

MAX = 9
ROOT = pathlib.Path(__file__).resolve().parent.parent / "src-tauri"
DIRS = [ROOT / "src", ROOT / "crates"]
SKIP = ("crates/cortex-sandbox/wasm/",)
MUTABLE = re.compile(
    r"\b(Mutex|FairMutex|RwLock|Atomic[A-Za-z0-9]*|Cell|RefCell|UnsafeCell|Condvar|Sender|"
    r"UnboundedSender|broadcast|mpsc|watch)\b"
)
STATIC = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?static\s+(?:mut\s+)?([A-Za-z_0-9]+)\s*:", re.M)


def without_tests(text):
    """The file up to its test module, if it has one at the end."""
    m = re.search(r"(?m)^#\[cfg\(test\)\]\s*\n\s*mod\s+\w+\s*\{", text)
    return text[: m.start()] if m else text


def mutable_names(text):
    """Type aliases and structs in this file that hold something mutable."""
    names = set()
    for m in re.finditer(r"(?s)\btype\s+(\w+)\s*=\s*(.*?);", text):
        if MUTABLE.search(m.group(2)):
            names.add(m.group(1))
    for m in re.finditer(r"(?s)\bstruct\s+(\w+)\s*\{(.*?)\n\}", text):
        if MUTABLE.search(m.group(2)):
            names.add(m.group(1))
    return names


def statics():
    found = []
    for base in DIRS:
        for path in sorted(base.rglob("*.rs")):
            rel = path.relative_to(ROOT).as_posix()
            if any(s in rel for s in SKIP) or "/target/" in rel:
                continue
            if path.name in ("tests.rs", "testing.rs") or path.stem.endswith("_tests"):
                continue
            text = without_tests(path.read_text(encoding="utf-8"))
            local = mutable_names(text)
            for m in STATIC.finditer(text):
                end = text.find("=", m.end())
                kind = text[m.end() : end if end != -1 else m.end() + 200]
                if not (MUTABLE.search(kind) or any(re.search(rf"\b{n}\b", kind) for n in local)):
                    continue
                line = text.count("\n", 0, m.start()) + 1
                above = text[: m.start()].split("\n")[-8:]
                if any("#[cfg(test)]" in l for l in above[-3:]):
                    continue  # a test's own, in a function only tests build
                reason = next(
                    (l.strip().lstrip("/!* ").strip() for l in above if "Process-wide:" in l), None
                )
                found.append((f"src-tauri/{rel}:{line}", m.group(1), reason))
    return found


def main():
    found = statics()
    if "--list" in sys.argv:
        for where, name, reason in found:
            print(f"{where}  {name}  {reason or '(no reason given)'}")
    missing = [f for f in found if not f[2]]
    for where, name, _ in missing:
        print(f"check-statics: {where}: {name} is a mutable static with no `Process-wide:` reason")
    if len(found) > MAX:
        print(f"check-statics: {len(found)} mutable statics, more than {MAX}; keep state in the app's state")
    if missing or len(found) > MAX:
        sys.exit(1)
    print(f"check-statics: {len(found)} mutable statics, each with a reason")


if __name__ == "__main__":
    main()
