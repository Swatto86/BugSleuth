import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

export function manifestVersion(file) {
  const value = JSON.parse(fs.readFileSync(file, "utf8")).version;
  if (typeof value !== "string" || value === "") {
    throw new Error(`no root version in ${file}`);
  }
  return value;
}

export function assertVersionAgreement(cargoVersion, root = process.cwd()) {
  const tauri = manifestVersion(
    path.join(root, "src-tauri", "tauri.conf.json"),
  );
  const npm = manifestVersion(path.join(root, "package.json"));
  if (!cargoVersion || cargoVersion !== tauri || cargoVersion !== npm) {
    throw new Error(
      `Cargo.toml ${cargoVersion}, tauri.conf.json ${tauri}, package.json ${npm}`,
    );
  }
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  try {
    assertVersionAgreement(process.argv[2] ?? "");
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  }
}
