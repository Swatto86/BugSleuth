/** Private app settings, repository and CLI fixture shared by both platforms. */
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFileSync } from "node:child_process";

const liveCodex = process.env["BUGSLEUTH_E2E_LIVE_CODEX"] === "1";
export const live = process.env["BUGSLEUTH_E2E_LIVE"] === "1" || liveCodex;
export function prepareWorkspace(root: string): void {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "bugsleuth-e2e-"));
  process.env["BUGSLEUTH_E2E_ROOT"] = dir;
  process.env["BUGSLEUTH_E2E_REPO"] = path.join(dir, "seeded-repo");
  process.env["APPDATA"] = path.join(dir, "appdata");
  const repo = process.env["BUGSLEUTH_E2E_REPO"];
  fs.cpSync(path.join(root, "fixtures/seeded-repo"), repo, {
    recursive: true,
    filter: (source) => ![".git", "target"].includes(path.basename(source)),
  });
  const git = (args: string[]) =>
    execFileSync("git", ["-C", repo, ...args], { stdio: "pipe" });
  git(["init", "-b", "main"]);
  git(["add", "."]);
  git([
    "-c",
    "user.name=BugSleuth",
    "-c",
    "user.email=acceptance@localhost",
    "commit",
    "-m",
    "Acceptance fixture",
  ]);
  if (!live || liveCodex) {
    // Codex also searches known installations under USERPROFILE before PATH.
    // Keep those lookups inside the fixture so it cannot launch a real account.
    if (!live) {
      process.env["USERPROFILE"] = path.join(dir, "home");
      fs.mkdirSync(process.env["USERPROFILE"], { recursive: true });
    }
    const bin = path.join(dir, "bin");
    fs.mkdirSync(bin);
    const fixture = path.join(root, "e2e/fixture-provider.mjs");
    const windows = process.platform === "win32";
    const script = windows
      ? `@echo off\r\n"${process.execPath}" "${fixture}" %*\r\n`
      : `#!/bin/sh\nexec '${process.execPath.replaceAll("'", "'\\''")}' '${fixture.replaceAll("'", "'\\''")}' "$@"\n`;
    for (const vendor of liveCodex
      ? ["opencode"]
      : ["opencode", "claude", "codex", "agent"]) {
      fs.writeFileSync(
        path.join(bin, windows ? `${vendor}.cmd` : vendor),
        script,
        { mode: 0o700 },
      );
    }
    process.env["PATH"] = `${bin}${path.delimiter}${process.env["PATH"] ?? ""}`;
    process.env["BUGSLEUTH_E2E_MODEL"] =
      "opencode:local-fixture/reviewer:latest";
  }
  const settingsDir = path.join(process.env["APPDATA"], "BugSleuth");
  fs.mkdirSync(settingsDir, { recursive: true });
  fs.writeFileSync(
    path.join(settingsDir, "settings.json"),
    JSON.stringify({
      repo,
      models: [
        {
          id: process.env["BUGSLEUTH_E2E_MODEL"] ?? "haiku",
          lanes: ["correctness"],
          effort: "",
          use_agents: false,
          passes: 1,
        },
      ],
      theme: "dark",
      triage_model: "",
      reuse_completed: false,
    }),
  );
}

/** Only inspect matching application processes; never kill by executable name. */
export function appPids(application: string): number[] {
  if (process.platform === "win32") {
    const script =
      "$p = $env:BUGSLEUTH_E2E_APPLICATION; Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -eq $p } | ForEach-Object { $_.ProcessId }";
    const result = execFileSync(
      "powershell.exe",
      ["-NoProfile", "-NonInteractive", "-Command", script],
      {
        encoding: "utf8",
        env: { ...process.env, BUGSLEUTH_E2E_APPLICATION: application },
      },
    );
    return result.split(/\s+/).filter(Boolean).map(Number);
  }
  return fs
    .readdirSync("/proc")
    .filter((name) => /^\d+$/.test(name))
    .flatMap((name) => {
      try {
        return fs.readlinkSync(`/proc/${name}/exe`) === application
          ? [Number(name)]
          : [];
      } catch (error) {
        if (
          ["ENOENT", "EACCES", "EPERM"].includes(
            (error as NodeJS.ErrnoException).code ?? "",
          )
        )
          return [];
        throw error;
      }
    });
}

export interface ProcessRow {
  pid: number;
  parent: number;
  args: string;
}
/** Match invocations only after proving descent from this test's app. */
export function providerDescendants(
  rows: ProcessRow[],
  roots: number[],
): number[] {
  const owned = new Set(roots);
  let changed = true;
  while (changed) {
    changed = false;
    for (const row of rows) {
      if (owned.has(row.parent) && !owned.has(row.pid)) {
        owned.add(row.pid);
        changed = true;
      }
    }
  }
  return rows
    .filter(
      (row) =>
        owned.has(row.pid) &&
        !roots.includes(row.pid) &&
        /(?:^|\s)--(?:agent|output-format|output-last-message|print)(?:\s|=|$)/.test(
          row.args,
        ),
    )
    .map((row) => row.pid);
}

export function providerPids(application: string): number[] {
  const roots = appPids(application);
  if (!roots.length) return [];
  let rows: ProcessRow[];
  if (process.platform === "win32") {
    const raw = execFileSync(
      "powershell.exe",
      [
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "@(Get-CimInstance Win32_Process | ForEach-Object { @{ pid = [int]$_.ProcessId; parent = [int]$_.ParentProcessId; args = [string]$_.CommandLine } }) | ConvertTo-Json -Compress",
      ],
      { encoding: "utf8" },
    );
    rows = JSON.parse(raw);
  } else {
    rows = fs
      .readdirSync("/proc")
      .filter((pid) => /^\d+$/.test(pid))
      .flatMap((pid) => {
        try {
          const status = fs.readFileSync(`/proc/${pid}/status`, "utf8");
          const parent = Number(status.match(/^PPid:\s+(\d+)/m)?.[1]);
          const args = fs
            .readFileSync(`/proc/${pid}/cmdline`, "utf8")
            .replaceAll("\0", " ");
          return [{ pid: Number(pid), parent, args }];
        } catch (error) {
          if (
            ["ENOENT", "EACCES", "EPERM", "ESRCH"].includes(
              (error as NodeJS.ErrnoException).code ?? "",
            )
          )
            return [];
          throw error;
        }
      });
  }
  return providerDescendants(rows, roots);
}
