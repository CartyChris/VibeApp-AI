# VibeApp AI

A native macOS app for building iPhone, iPad, Android, web and desktop apps by describing them, and for **remaking existing apps and websites for other devices**.

Rust core on [Tauri 2](https://tauri.app) with the system WebKit view. No Electron, no bundler, no frontend framework. The app is about 7 MB.

## What it does

- **Build by describing.** Chat with any of 400+ models (OpenRouter, Anthropic, OpenAI, Google, Z.ai, DeepSeek, Groq, or local Ollama / LM Studio / llama.cpp). The app is shown live in a device simulator with real safe areas.
- **Remake an app.** Point it at a website, a source folder, a Mac `.app` (Electron apps included), an iOS `.ipa` or an Android `.apk`. It inspects the original without running it and writes a port brief listing:
  - the screens and features
  - its permissions, document types and URL schemes
  - the endpoints it calls
  - its design tokens and UI text

  It then starts a project for iPhone, iPad, Android, the web or the desktop.
- **Tools over MCP.** Models can call tools on MCP servers you connect: Ghidra (through REA, ReVa or GhidraMCP), radare2, JADX, Playwright, Chrome DevTools, GitHub, Context7, Figma and more. You approve every call unless you choose to trust a tool or server. You can also import the servers you've already set up for Claude Desktop, Claude Code, Cursor or Windsurf.
- **Skills.** These are instruction packs loaded only when they're relevant. Built-in packs cover the iPhone safe area, game loops, offline use, accessibility, clean-room remakes, reverse-engineering triage, desktop-to-phone and website-to-app ports, and tablet layouts. One click imports more from GitHub, including the ReVa and REA reverse-engineering packs and Anthropic's skills.
- **Safe edits.** Changes can apply live, wait for your review, or run in a sandbox copy. Every turn has undo and redo, and you can rewind to any earlier message. A crash journal means a long reply is never lost.
- **Audit.** Checks accessibility, credentials, offline use, performance, localisation, ship-readiness, and phone and tablet readiness. Each finding comes with a one-click fix.
- **Swarms and Arena.** Several models work blind to one another's identity, with gates between stages. Or you can send one prompt to several models and adopt the best result.
- **Export.** Xcode project, PWA, Electron, Android project, plus iOS Simulator runs.

## Trust model

- API keys and MCP secrets are kept only in the macOS Keychain. They are never written to disk or a log.
- Model output is untrusted.
  - A model never runs a shell command: `<vibe:shell>` is shown to you, never executed.
  - It can't write outside the project.
  - It can only pick a tool and its arguments on an MCP server you defined. It never chooses a command line.
- Inspecting an app to remake it is static: nothing is executed and nothing is uploaded.

## Build

Requirements: macOS, Rust (stable), Node 18+, and the Xcode command-line tools.

```bash
npm install
python3 tools/check.py              # frontend checks, Rust tests and invariants
npx tauri build --bundles app       # builds src-tauri/target/release/bundle/macos/VibeApp AI.app
./tools/package_dmg.sh              # signs (ad hoc) and writes a DMG to ~/Downloads
```

Read [ARCHITECTURE.md](ARCHITECTURE.md) before changing anything. It explains the output protocol and the orchestration loop, lists every bug already fixed (each guarded by a test), and gives the invariants that must hold. [AGENT_PROMPT.md](AGENT_PROMPT.md) is the brief for an AI agent continuing the work.
