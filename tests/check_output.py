#!/usr/bin/env python3
"""Renders documents with the native binary and checks what Tern would receive.

For every part of every rendered file:
  - nodes are well-formed (`k` string, `p` object, `c` list of nodes);
  - `el` tags are ones Tern draws, `attrs` keys are on Tern's allowlist;
  - every class an `el` carries is defined by office.css / chrome.css (no silent no-op class);
  - every image node's `src` is listed in the part's `images` and exists on disk;
  - parts stay below the node budget.

Usage: tests/check_output.py [FILE...]   (default: tests/fixtures/*.docx|*.xlsx)
       OFFICE_PREVIEW_BIN=path overrides the binary (default: release build in /tmp target).
"""

import json
import os
import pathlib
import re
import subprocess
import sys
import tempfile
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent
TARGET = os.environ.get("CARGO_TARGET_DIR", "/tmp/office-preview-target")
TAGS = set(
    "div span p section header footer nav aside main article figure blockquote ul ol li dl dt dd "
    "h1 h2 h3 h4 pre code kbd strong b em i del mark hr table thead tbody tr th td label button form".split()
)
ATTR = re.compile(r"^(colspan|rowspan|role|aria-labelledby|(data|aria)-[a-z0-9_-]+)$")
KINDS = {"el", "image", "math", "text", "col", "row"}
MAX_NODES = 8000


def binary() -> str:
    env_bin = os.environ.get("OFFICE_PREVIEW_BIN")
    if env_bin:
        return env_bin
    subprocess.run(
        ["cargo", "build", "--release", "--quiet"],
        cwd=ROOT / "native",
        env={**os.environ, "CARGO_TARGET_DIR": TARGET},
        check=True,
    )
    return f"{TARGET}/release/office-preview"


def defined_classes() -> set[str]:
    css = (ROOT / "office.css").read_text() + (ROOT / "chrome.css").read_text()
    return set(re.findall(r"\.([A-Za-z_][\w-]*)", css))


def check_node(node, where: str, classes: set[str], images: set[str], errors: list[str], count: list[int]):
    count[0] += 1
    if not isinstance(node, dict) or not isinstance(node.get("k"), str):
        errors.append(f"{where}: not a node: {str(node)[:80]}")
        return
    k = node["k"]
    p = node.get("p", {})
    if not isinstance(p, dict):
        errors.append(f"{where}: props not an object")
        return
    if k not in KINDS:
        errors.append(f"{where}: unexpected kind {k}")
    if k == "el":
        tag = p.get("tag")
        if tag not in TAGS:
            errors.append(f"{where}: el tag {tag!r} is not drawn by Tern")
        for a in (p.get("attrs") or {}):
            if not ATTR.match(a):
                errors.append(f"{where}: attr {a!r} is not allowed")
        for c in str(p.get("class", "")).split():
            if c not in classes:
                errors.append(f"{where}: class {c!r} is not defined in office.css")
        if "text" in p and not isinstance(p["text"], str):
            errors.append(f"{where}: text is not a string")
    if k == "image":
        src = p.get("src")
        if src not in images:
            errors.append(f"{where}: image src {src!r} not listed in part images")
    if k == "math" and not isinstance(p.get("text"), str):
        errors.append(f"{where}: math without text")
    children = node.get("c", [])
    if not isinstance(children, list):
        errors.append(f"{where}: children not a list")
        return
    for i, c in enumerate(children):
        check_node(c, f"{where}.{i}", classes, images, errors, count)


def check(path: pathlib.Path, bin_path: str, cache: str, classes: set[str]) -> list[str]:
    t0 = time.monotonic()
    r = subprocess.run([bin_path, "render", str(path), "--cache", cache], capture_output=True, text=True)
    took = time.monotonic() - t0
    try:
        out = json.loads(r.stdout)
    except json.JSONDecodeError:
        return [f"{path.name}: no JSON on stdout (status {r.returncode}): {r.stderr[-400:]}"]
    if not out.get("ok"):
        return [f"{path.name}: render failed: {out.get('error')}"]
    meta = out["meta"]
    d = pathlib.Path(out["dir"])
    if meta["kind"] == "docx":
        parts = [p["file"] for p in meta["parts"]]
    else:
        parts = [p["file"] for s in meta["sheets"] for p in s["parts"]]
    errors: list[str] = []
    if not parts:
        errors.append(f"{path.name}: no parts")
    total = 0
    for f in parts:
        doc = json.loads((d / f"{f}.json").read_text())
        images = {i["file"] for i in doc.get("images", [])}
        for i in images:
            if not (d / i).is_file():
                errors.append(f"{path.name}/{f}: missing image file {i}")
        count = [0]
        check_node(doc["node"], f"{path.name}/{f}", classes, images, errors, count)
        total += count[0]
        if count[0] > MAX_NODES:
            errors.append(f"{path.name}/{f}: {count[0]} nodes (budget {MAX_NODES})")
    print(f"{'ok' if not errors else 'FAIL':4} {path.name}: {meta['kind']}, {len(parts)} parts, {total} nodes, {took:.2f}s")
    return errors


def main() -> int:
    files = [pathlib.Path(a) for a in sys.argv[1:]] or sorted(
        p for p in (ROOT / "tests" / "fixtures").glob("*") if p.suffix in (".docx", ".xlsx")
    )
    if not files:
        print("no input files")
        return 1
    bin_path = binary()
    classes = defined_classes()
    errors: list[str] = []
    with tempfile.TemporaryDirectory() as cache:
        for f in files:
            errors += check(f, bin_path, cache, classes)
    for e in errors[:60]:
        print("  " + e)
    if len(errors) > 60:
        print(f"  … {len(errors) - 60} more")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
