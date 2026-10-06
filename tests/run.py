#!/usr/bin/env python3
"""Runs the plugin's Luau tests under the standalone `luau` CLI.

The CLI sandboxes each required module's globals, so the plugin's free `tern` can't be faked
through `require`. This script bundles every module into one chunk instead: each module becomes
a factory over a shared `tern` upvalue, and `__fresh(env)` swaps in a new fake runtime and drops
the module cache so every scenario starts from clean module state.

Usage: tests/run.py [test files...]   (default: every tests/test_*.luau)
"""

import pathlib
import re
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
REQUIRE = re.compile(r'require\("(\.\.?/[^"]+)"\)')


def key_of(path: pathlib.Path) -> str:
    return path.relative_to(ROOT).with_suffix("").as_posix()


def resolve(base: pathlib.Path, spec: str) -> pathlib.Path:
    target = (base.parent / spec).resolve()
    for cand in (target.with_suffix(".luau"), target / "init.luau"):
        if cand.exists():
            return cand
    raise SystemExit(f"cannot resolve {spec} from {base}")


def bundle(test: pathlib.Path) -> str:
    factories: dict[str, str] = {}

    def rewrite(path: pathlib.Path, text: str) -> str:
        def sub(m: re.Match) -> str:
            dep = resolve(path, m.group(1))
            load(dep)
            return f'require("{key_of(dep)}")'

        text = REQUIRE.sub(sub, text)
        # Exported types are only legal at a chunk's top level; inside a factory they're local.
        return re.sub(r"^export type ", "type ", text, flags=re.M)

    def load(path: pathlib.Path) -> None:
        key = key_of(path)
        if key in factories:
            return
        factories[key] = ""
        factories[key] = rewrite(path, path.read_text())

    test_src = rewrite(test, test.read_text())
    parts = [
        "local tern: any = nil",
        "local __factories = {}",
        "local __cache = {}",
        "local function require(key: string): any",
        "\tif key == 'tern' then return tern end",
        "\tif __cache[key] == nil then __cache[key] = __factories[key]() end",
        "\treturn __cache[key]",
        "end",
        "local function __fresh(env: any) tern = env.tern; table.clear(__cache) end",
    ]
    for key, src in factories.items():
        parts.append(f'__factories["{key}"] = function()\n{src}\nend')
    parts.append(test_src)
    return "\n".join(parts)


def main() -> int:
    tests = [pathlib.Path(a).resolve() for a in sys.argv[1:]] or sorted((ROOT / "tests").glob("test_*.luau"))
    failed = 0
    for test in tests:
        with tempfile.NamedTemporaryFile("w", suffix=".luau", delete=False) as f:
            f.write(bundle(test))
            out = f.name
        r = subprocess.run(["luau", out], capture_output=True, text=True)
        status = "ok" if r.returncode == 0 else "FAIL"
        print(f"{status:4} {test.relative_to(ROOT)}")
        if r.stdout.strip():
            print(r.stdout.rstrip())
        if r.returncode != 0:
            failed += 1
            print(r.stderr.rstrip())
            print(f"     bundle kept at {out}")
        else:
            pathlib.Path(out).unlink()
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
