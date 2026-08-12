# BugSleuth — AI Context

## System Overview

BugSleuth runs multi-lane code reviews through signed-in coding CLIs (not metered HTTP APIs), verifies every finding against the repository, then judges agreement across models. Two front ends share one engine: `bugsleuth-cli` and the Tauri desktop app.

## Tech Stack & Architecture

Rust workspace (`bugsleuth-domain`, `provider`, `verify`, `judge`, `engine`, `cli`) plus Tauri (`src-tauri`) and a vanilla TypeScript UI (`ui/`). Vendor differences live only in `bugsleuth-provider` as a closed `Vendor` enum, not a trait.

## Component Map

- `crates/bugsleuth-provider/src/{claude,codex,kilo,kimi,cursor}.rs` — one subprocess adapter per CLI
- `crates/bugsleuth-engine/src/sweep.rs` — `Vendor::parse`, isolation, invoke
- `crates/bugsleuth-engine/src/sweep/isolate.rs` — strip project instruction files from throwaway worktrees
- `src-tauri/src/catalogue.rs` + `ui/src/model.ts` — vendor menus (`VENDORS` must stay in step)

```mermaid
flowchart LR
  UI[ui / cli] --> Engine[bugsleuth-engine]
  Engine --> Provider[bugsleuth-provider]
  Provider --> Claude[claude]
  Provider --> Codex[codex]
  Provider --> Kilo[kilo]
  Provider --> Kimi[kimi]
  Provider --> Cursor[cursor / agent]
  Engine --> Verify[bugsleuth-verify]
  Engine --> Judge[bugsleuth-judge]
```

## Data Flow

Config (`vendor:model` specs) → plan units → per-vendor precheck → isolated sweep → anchor verify → judge → report / apply handoff.

Cursor specs look like `cursor:composer-2.5`. The CLI binary users type is `agent`; BugSleuth prefers the install tree's `node.exe` + `index.js` over the Windows `.cmd` shim. Sweeps use `-p --mode ask --trust`; applies use `-p --force --trust`.

## Recent Context & Decisions

- 2026-08-12: Added Cursor Agent CLI (`agent`) as vendor `cursor`. Ask mode is the write boundary; worktree isolation remains because there is no ignore-rules flag. Effort is encoded in model ids (no separate effort flag). No schema enforcement — brief describes JSON shape.
