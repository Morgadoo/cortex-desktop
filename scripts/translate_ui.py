#!/usr/bin/env python3
"""Fills src-tauri/i18n/<code>.json with translations of the interface catalog.

Run after `node scripts/ui-strings.mjs` has updated src-tauri/i18n/catalog.json. Only strings
not yet translated are sent. Uses any OpenAI-compatible chat endpoint, from the environment:

    CORTEX_TRANSLATE_BASE_URL   e.g. https://api.openai.com/v1
    CORTEX_TRANSLATE_API_KEY    the key (never written anywhere)
    CORTEX_TRANSLATE_MODEL      e.g. gpt-4.1-mini

    python3 scripts/translate_ui.py pt es fr de
"""
import concurrent.futures
import json
import os
import re
import sys
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent / "src-tauri" / "i18n"
NAMES = {"pt": "European Portuguese", "es": "Spanish (Spain)", "fr": "French", "de": "German", "it": "Italian", "nl": "Dutch", "pl": "Polish", "ja": "Japanese", "zh": "Simplified Chinese", "ar": "Modern Standard Arabic", "he": "Hebrew"}
BATCH = 80


def placeholders(s):
    return sorted(re.findall(r"\{\d+\}", s))


def ask(prompt):
    body = json.dumps({"model": os.environ["CORTEX_TRANSLATE_MODEL"], "messages": [{"role": "user", "content": prompt}], "temperature": 0.2}).encode()
    req = urllib.request.Request(os.environ["CORTEX_TRANSLATE_BASE_URL"].rstrip("/") + "/chat/completions", data=body, headers={"Content-Type": "application/json", "Authorization": "Bearer " + os.environ["CORTEX_TRANSLATE_API_KEY"]})
    with urllib.request.urlopen(req, timeout=300) as r:
        return json.load(r)["choices"][0]["message"]["content"]


def translate_batch(code, batch):
    prompt = (
        f"Translate these user-interface strings of a desktop AI assistant app called Cortex from English into {NAMES[code]}. "
        "Use the usual words of desktop software in that language and the same tone (short, plain, friendly); keep each "
        "string's capitalisation style and punctuation. Keep {0}-style placeholders, product names (Cortex, Markdown, MCP, "
        "OpenAPI, GitHub, Python, Ctrl, Tab, Esc…), keyboard keys and code exactly as they are. Strings that are only "
        "search keywords may be translated as keywords. Reply with one JSON object mapping each original string to its "
        "translation, and nothing else.\n\n" + json.dumps(batch, ensure_ascii=False, indent=1)
    )
    for _ in range(3):
        try:
            reply = ask(prompt)
            got = json.loads(reply[reply.index("{"): reply.rindex("}") + 1])
            return {k: v for k, v in got.items() if k in batch and isinstance(v, str) and v.strip() and placeholders(k) == placeholders(v)}
        except Exception as e:  # noqa: BLE001 - retried, then reported
            err = e
    print(f"{code}: a batch failed: {err}", file=sys.stderr)
    return {}


def run(code):
    catalog = json.loads((ROOT / "catalog.json").read_text())
    path = ROOT / f"{code}.json"
    known = json.loads(path.read_text()) if path.exists() else {}
    known = {k: v for k, v in known.items() if k in catalog}
    missing = [s for s in catalog if s not in known]
    batches = [missing[i:i + BATCH] for i in range(0, len(missing), BATCH)]
    with concurrent.futures.ThreadPoolExecutor(max_workers=int(os.environ.get("CORTEX_TRANSLATE_WORKERS", "2"))) as pool:
        for got in pool.map(lambda b: translate_batch(code, b), batches):
            known.update(got)
    path.write_text(json.dumps(dict(sorted(known.items())), ensure_ascii=False, indent=1) + "\n")
    print(f"{code}: {sum(1 for s in catalog if s in known)} of {len(catalog)}")


if __name__ == "__main__":
    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
        list(pool.map(run, sys.argv[1:]))
