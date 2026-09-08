// A subprocess fixture, not an app-side IPC mock. The real engine must parse,
// anchor-verify and persist this finding from its actual isolated working tree.
import fs from "node:fs";
import path from "node:path";
const args = process.argv.slice(2);
if (args.includes("--version")) {
  console.log("1.18.29-fixture");
} else if (args.includes("models")) {
  console.log('local-fixture/reviewer:latest\n{"variants":{"thinking":{}}}');
} else if (args[0] === "run") {
  let prompt = "";
  for await (const chunk of process.stdin) prompt += chunk;
  await new Promise((resolve) => setTimeout(resolve, 2000));
  const editing = writable();
  if (editing) await pauseApply();
  const text = prompt.includes("Reply with exactly OK and nothing else.")
    ? "OK"
    : editing ? apply() : review();
  console.log(JSON.stringify({ type: "text", part: { id: "part", messageID: "answer", text } }));
} else if (args.includes("--print") && args.includes("Read,Glob,Grep,Edit,Write,Bash")) {
  for await (const chunk of process.stdin) { /* consume the real prompt */ }
  await pauseApply();
  console.log(JSON.stringify({ type: "result", result: apply(), is_error: false }));
} else {
  console.error("Unexpected fixture invocation");
  process.exitCode = 1;
}
function review() {
  const source = fs.readFileSync(path.join(process.cwd(), "src/pricing.rs"), "utf8");
  const lines = source.split(/\r?\n/);
  const line = lines.findIndex((text) => text.includes("if quantity > 50"));
  if (line < 0) throw new Error("The fixture is missing its known defect");
  return JSON.stringify({ findings: [{
    title: "Bulk discount excludes the advertised threshold",
    severity: "medium", file: "src/pricing.rs", line: line + 1,
    snippet: lines[line],
    explanation: "The comparison excludes a basket containing exactly fifty items.",
    failure_scenario: "basket_total(100, 50) returns 4750 instead of 4500.",
  }] });
}

function writable() {
  const config = JSON.parse(process.env.OPENCODE_CONFIG_CONTENT);
  return Object.values(config.agent).some((agent) => agent.permission.edit === "allow");
}
function apply() {
  const file = path.join(process.cwd(), "src/pricing.rs");
  const source = fs.readFileSync(file, "utf8");
  if (!source.includes("if quantity > 50")) throw new Error("Expected original threshold");
  fs.writeFileSync(file, source.replace("if quantity > 50", "if quantity >= 50"));
  return `Fixed the bulk discount threshold in src/pricing.rs for ${path.basename(process.cwd())}.`;
}

async function pauseApply() {
  const root = path.dirname(process.cwd());
  if (!root || !fs.existsSync(path.join(root, "parallel-apply"))) return;
  fs.appendFileSync(path.join(root, "applies.jsonl"), JSON.stringify({ repo: process.cwd(), args, at: Date.now() }) + "\n");
  await new Promise(resolve => setTimeout(resolve, args[0] === "run" ? 30000 : 20000));
}
