#!/usr/bin/env python3
"""Everything that must pass before a build, in one command.

    python3 tools/check.py

Runs the frontend import/syntax check, the Rust test suite, and a handful of
invariant greps that catch mistakes neither of those would:

  * a Tauri command defined but never registered in `generate_handler!`
    (it compiles, and then fails at runtime with "command not found")
  * a hard-coded colour in app.css (breaks every theme)
  * a code path that executes `<vibe:shell>` (model output is untrusted)
  * a flat context budget constant (this is how bug #5 shipped)
"""
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def run(label, cmd, cwd=ROOT):
    print(f"\n── {label} " + "─" * max(0, 58 - len(label)), flush=True)
    r = subprocess.run(cmd, cwd=cwd, shell=isinstance(cmd, str))
    return r.returncode == 0


def unregistered_commands():
    """Tauri commands that exist but were never added to generate_handler!."""
    lib = open(os.path.join(ROOT, "src-tauri/src/lib.rs")).read()
    defined = re.findall(
        r"#\[tauri::command\]\s*(?:pub\s+)?(?:async\s+)?fn\s+([a-z_0-9]+)", lib)
    m = re.search(r"generate_handler!\[(.*?)\]", lib, re.S)
    if not m:
        return ["generate_handler! not found at all"]
    registered = set(re.findall(r"[a-z_0-9]+", m.group(1)))
    return [f"{c} is a #[tauri::command] but is not in generate_handler!"
            for c in defined if c not in registered]


def invariant_greps():
    problems = []
    css = open(os.path.join(ROOT, "src/app.css")).read()
    for n, line in enumerate(css.splitlines(), 1):
        if re.match(r"\s*(color|background|border-color)\s*:\s*#[0-9a-fA-F]{3,8}", line):
            # The simulator's physical device colours are deliberate and
            # commented as such; everything else must be a token.
            if "#000" not in line and "#fff" not in line:
                problems.append(f"app.css:{n}: hard-coded colour — use a token")

    src = os.path.join(ROOT, "src-tauri/src")
    for f in os.listdir(src):
        if not f.endswith(".rs"):
            continue
        body = open(os.path.join(src, f)).read()
        # MCP servers and app inspection start programs the user named,
        # directly. A shell would turn an argument into a command line.
        if f in ("mcp.rs", "port.rs", "bridges.rs") and re.search(
                r'Command::new\(\s*"(?:/bin/)?(?:sh|bash|zsh)"', body):
            problems.append(f"{f}: starts a process through a shell")
        # A Suggest op must never reach a process spawn.
        if re.search(r"FileOp::Suggest\s*\{[^}]*\}\s*=>\s*[^,\n]*Command::new", body):
            problems.append(f"{f}: a suggested shell command reaches Command::new")

    agent = open(os.path.join(src, "agent.rs")).read()
    if "model_context_tokens" not in agent:
        problems.append("agent.rs: context_budget no longer derives from the model window")
    return problems


EMOJI = re.compile("[\U0001F000-\U0001FAFF\u2600-\u26FF\u2700-\u27BF\u2B50\u2B55\u23E9-\u23FA\uFE0F\u20E3]")
# Plain text glyphs that render in the interface font, not the emoji font.
TEXT_SYMBOLS = set("\u2713\u2715\u2605")


def interface_files():
    src = os.path.join(ROOT, "src")
    for base, _, names in os.walk(src):
        for n in names:
            if n.endswith((".js", ".html", ".css")):
                yield os.path.join(base, n)


def emoji_problems():
    """The interface draws its symbols as SVG, so no emoji may reach it."""
    out = []
    for path in interface_files():
        for n, line in enumerate(open(path, encoding="utf8"), 1):
            found = [c for c in EMOJI.findall(line) if c not in TEXT_SYMBOLS]
            if found:
                out.append(f"{os.path.relpath(path, ROOT)}:{n}: emoji {''.join(found)} — use an icon")
    return out


def icon_problems():
    """A misspelt icon name draws a dot, which nothing else would catch."""
    js = os.path.join(ROOT, "src", "js")
    body = open(os.path.join(js, "icons.js"), encoding="utf8").read()
    known = set(re.findall(r"^\s+([a-zA-Z]+):\s*'", body, re.M))
    if len(known) < 20:
        return ["icons.js: could not read the icon table"]
    out = []
    patterns = [
        r"\bicon\('([a-zA-Z]+)'\)",
        r"\bemptyState\('([a-zA-Z]+)'",
        r"\bicon: '([a-zA-Z]+)'",
        r"\bcmd\('([a-zA-Z]+)'",
    ]
    for f in sorted(os.listdir(js)):
        if not f.endswith(".js"):
            continue
        src = open(os.path.join(js, f), encoding="utf8").read()
        for pat in patterns:
            for name in re.findall(pat, src):
                if name not in known:
                    out.append(f"{f}: icon '{name}' does not exist in icons.js")
    # Objects mapping something to an icon name, e.g. KIND_ICON, ROLE_ICON.
    for f, table in [("panels3.js", "TARGET_ICON"), ("panels4.js", "KIND_ICON"), ("panels6.js", "ROLE_ICON")]:
        src = open(os.path.join(js, f), encoding="utf8").read()
        m = re.search(table + r" = \{(.*?)\}", src, re.S)
        if not m:
            out.append(f"{f}: {table} not found")
            continue
        for name in re.findall(r":\s*'([a-zA-Z]+)'", m.group(1)):
            if name not in known:
                out.append(f"{f}: {table} names '{name}', which does not exist")
    return out


def native_dialog_problems():
    """WKWebView does not implement these: they return at once and show nothing."""
    out = []
    pat = re.compile(r"(?<![\w.$])(?:window\.)?(prompt|confirm|alert)\(")
    for path in interface_files():
        if not path.endswith(".js"):
            continue
        for n, line in enumerate(open(path, encoding="utf8"), 1):
            if pat.search(line) and "await ask" not in line:
                out.append(f"{os.path.relpath(path, ROOT)}:{n}: native dialog — use askText/askConfirm")
    return out


def mount_problems():
    """A mount() that returns nothing breaks panel switching for the whole rail."""
    js = os.path.join(ROOT, "src", "js")
    out = []
    for f in sorted(os.listdir(js)):
        if not f.startswith("panels") and f != "appearance.js":
            continue
        src = open(os.path.join(js, f), encoding="utf8").read()
        mounts = len(re.findall(r"^\s+mount\(host\)", src, re.M))
        returns = len(re.findall(r"return root;", src))
        if returns < mounts:
            out.append(f"{f}: {mounts} mount() but only {returns} 'return root'")
    return out


def main():
    ok = True
    ok &= run("frontend", [sys.executable, "tools/check_frontend.py"])
    ok &= run("rust tests", "cargo test --lib", cwd=os.path.join(ROOT, "src-tauri"))

    print("\n── invariants " + "─" * 46)
    problems = (unregistered_commands() + invariant_greps() + emoji_problems()
                + icon_problems() + native_dialog_problems() + mount_problems())
    if problems:
        ok = False
        for p in problems:
            print(f"  - {p}")
    else:
        print("  all invariants hold")

    print("\n" + ("ALL CHECKS PASSED" if ok else "CHECKS FAILED"))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
