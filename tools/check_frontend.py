#!/usr/bin/env python3
"""Static sanity check for the bundler-free frontend.

There is no build step, so a named import that no longer exists fails silently
at load time and simply makes panels vanish from the rail. This catches that
before it ships. Run it before every `tauri build`.

    python3 tools/check_frontend.py
"""
import os
import re
import subprocess
import sys
import tempfile

ROOT = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "src", "js")

IMPORT_RE = re.compile(
    r"import\s*\{([^}]*)\}\s*from\s*['\"]\./([\w.]+)\.js['\"]", re.S)
EXPORT_RE = re.compile(
    r"export\s+(?:async\s+)?(?:function|const|let|class)\s+([A-Za-z_$][\w$]*)")
EXPORT_LIST_RE = re.compile(r"export\s*\{([^}]*)\}")


def exports_of(path):
    src = open(path).read()
    names = set(EXPORT_RE.findall(src))
    for block in EXPORT_LIST_RE.findall(src):
        for part in block.split(","):
            part = part.strip()
            if not part:
                continue
            # `foo as bar` exports bar
            names.add(part.split(" as ")[-1].strip())
    return names


def main():
    files = sorted(f for f in os.listdir(ROOT) if f.endswith(".js"))
    if not files:
        print("no modules found", file=sys.stderr)
        return 1

    exports = {f[:-3]: exports_of(os.path.join(ROOT, f)) for f in files}
    problems = []

    # 1. every named import resolves to a real export
    for f in files:
        src = open(os.path.join(ROOT, f)).read()
        for names, module in IMPORT_RE.findall(src):
            if module not in exports:
                problems.append(f"{f}: imports from unknown module ./{module}.js")
                continue
            for raw in names.split(","):
                name = raw.strip().split(" as ")[0].strip()
                if not name:
                    continue
                if name not in exports[module]:
                    problems.append(
                        f"{f}: imports `{name}` from {module}.js, which does not export it")

    # 2. every module still parses
    with tempfile.TemporaryDirectory() as tmp:
        for f in files:
            mjs = os.path.join(tmp, f[:-3] + ".mjs")
            with open(mjs, "w") as out:
                out.write(open(os.path.join(ROOT, f)).read())
            r = subprocess.run(["node", "--check", mjs],
                               capture_output=True, text=True)
            if r.returncode != 0:
                first = (r.stderr.strip().splitlines() or ["syntax error"])[0]
                problems.append(f"{f}: {first}")

    if problems:
        print("FRONTEND CHECK FAILED\n")
        for p in problems:
            print(f"  - {p}")
        return 1

    total = sum(len(v) for v in exports.values())
    print(f"frontend OK — {len(files)} modules, {total} exports, all imports resolve")
    return 0


if __name__ == "__main__":
    sys.exit(main())
