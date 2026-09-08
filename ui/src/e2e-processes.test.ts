import { strict as assert } from "node:assert";
import { test } from "node:test";
import { providerDescendants } from "../../e2e/workspace.ts";

test("native cancellation finds every provider only inside the owned app process tree", () => {
  const rows = [
    { pid: 8, parent: 1, args: "claude --print --output-format json" },
    { pid: 100, parent: 1, args: "bugsleuth-app" },
    { pid: 104, parent: 103, args: "node agent --output-format json" },
    { pid: 101, parent: 100, args: "claude --print --output-format json" },
    { pid: 102, parent: 100, args: "codex exec --output-last-message answer" },
    { pid: 103, parent: 100, args: "cmd /C agent.cmd" },
    { pid: 105, parent: 100, args: "opencode run --agent private-review" },
    { pid: 106, parent: 100, args: "WebKitWebProcess" },
    { pid: 107, parent: 8, args: "opencode run --agent unrelated" },
  ];
  assert.deepEqual(
    providerDescendants(rows, [100]).sort(),
    [101, 102, 104, 105],
  );
  assert.deepEqual(providerDescendants(rows, []), []);
});
