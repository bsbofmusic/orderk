import * as fs from "node:fs";
import * as path from "node:path";

export function resolveOrderkBinary(explicitPath?: string): string {
  const candidates = [
    explicitPath,
    process.env.ORDERK_BIN,
    resolvePackageLocalBinary(),
    "orderk",
  ].filter(Boolean) as string[];

  for (const candidate of candidates) {
    if (candidate === "orderk") return candidate;
    if (fs.existsSync(candidate)) return candidate;
  }
  throw new Error("orderk CLI not found. Install `cargo run -p orderk-cli --bin orderk` or set ORDERK_BIN.");
}

function resolvePackageLocalBinary(): string | undefined {
  const entrypoint = process.argv[1] ? path.resolve(process.argv[1]) : process.cwd();
  const packageRoot = path.resolve(path.dirname(entrypoint), "..");
  const packageVendor = path.join(packageRoot, "vendor", process.platform === "win32" ? "orderk.exe" : "orderk");
  const repoRoot = path.resolve(packageRoot, "..", "..");
  const releaseBinary = path.join(repoRoot, "target", "release", process.platform === "win32" ? "orderk.exe" : "orderk");
  const debugBinary = path.join(repoRoot, "target", "debug", process.platform === "win32" ? "orderk.exe" : "orderk");
  return [packageVendor, releaseBinary, debugBinary].find((p) => fs.existsSync(p));
}
