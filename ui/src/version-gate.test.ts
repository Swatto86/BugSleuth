import { strict as assert } from "node:assert";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { test } from "node:test";

const root = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
);

test("version agreement reads only root manifest versions", async (t) => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "bugsleuth-version-"));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  const tauri = path.join(dir, "src-tauri", "tauri.conf.json");
  const npm = path.join(dir, "package.json");
  fs.mkdirSync(path.dirname(tauri));
  const decoy = JSON.stringify({
    metadata: { version: "0.2.51" },
    version: "9.9.9",
  });
  fs.writeFileSync(tauri, decoy);
  fs.writeFileSync(npm, decoy);

  const checker = (await import(
    pathToFileURL(path.join(root, "scripts", "check-version-agreement.mjs"))
      .href
  )) as {
    manifestVersion: (file: string) => string;
    assertVersionAgreement: (cargoVersion: string, root: string) => void;
  };
  assert.equal(checker.manifestVersion(tauri), "9.9.9");
  assert.throws(() => checker.assertVersionAgreement("0.2.51", dir));
});
