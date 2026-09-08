// Build and merge signed update metadata for exactly the artifacts published.
import fs from "node:fs";
import path from "node:path";
const [mode, directory, version, ...args] = process.argv.slice(2);
const supported = ["windows-x86_64", "linux-x86_64", "darwin-aarch64"];
if (!directory || !/^\d+\.\d+\.\d+(?:-[\w.-]+)?$/.test(version ?? "")) throw new Error("Expected output directory and release version");
const base = `https://github.com/Swatto86/BugSleuth/releases/download/v${version}/`;
const write = (file, value) => fs.writeFileSync(path.join(directory, file), JSON.stringify(value, null, 2) + "\n");
if (mode === "fragment") {
  const [platform, bundle, signatureFile] = args;
  if (!supported.includes(platform)) throw new Error(`Unsupported platform: ${platform}`);
  if (signatureFile !== `${bundle}.sig`) throw new Error("Signature must belong to the selected bundle");
  const signature = fs.readFileSync(signatureFile, "utf8").trim();
  if (!signature) throw new Error("Missing update signature");
  const asset = path.basename(bundle);
  if (!fs.statSync(bundle).isFile()) throw new Error("Update bundle is not a file");
  fs.copyFileSync(bundle, path.join(directory, asset));
  write(`latest-${platform}.json`, { version, platforms: { [platform]: { signature, url: base + encodeURIComponent(asset) } } });
} else if (mode === "merge") {
  const required = (args[0] ?? "").split(",");
  if (!required.length || required.some((name) => !supported.includes(name))) throw new Error("Specify the platforms this release builds");
  const platforms = {};
  for (const file of fs.readdirSync(directory).filter((name) => /^latest-.+\.json$/.test(name))) {
    const fragment = JSON.parse(fs.readFileSync(path.join(directory, file), "utf8"));
    if (fragment.version !== version || !fragment.platforms || typeof fragment.platforms !== "object") throw new Error(`Wrong release metadata: ${file}`);
    for (const [platform, entry] of Object.entries(fragment.platforms)) {
      if (!required.includes(platform) || platforms[platform]) throw new Error(`Unexpected or duplicate platform: ${platform}`);
      if (!entry || typeof entry.signature !== "string" || !entry.signature.trim() || typeof entry.url !== "string" || !entry.url.startsWith(base)) throw new Error(`Invalid update entry: ${platform}`);
      const asset = decodeURIComponent(entry.url.slice(base.length));
      if (asset !== path.basename(asset) || asset.includes("\\") || !fs.statSync(path.join(directory, asset)).isFile()) throw new Error(`Missing update artifact: ${platform}`);
      platforms[platform] = entry;
    }
  }
  if (required.some((platform) => !platforms[platform])) throw new Error("A required platform has no signed update");
  write("latest.json", { version, pub_date: new Date().toISOString(), platforms });
} else {
  throw new Error("Use fragment or merge");
}
