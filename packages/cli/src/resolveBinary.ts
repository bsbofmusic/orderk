import * as fs from "node:fs";
import * as path from "node:path";

export function resolveOrderkBinary(explicitPath?: string): string {
  const candidates = [
    explicitPath,
    process.env.ORDERK_BIN,
    resolvePackageLocalBinary(),
    resolvePathBinary(),
  ].filter(Boolean) as string[];

  for (const candidate of candidates) {
    if (fs.existsSync(candidate)) return candidate;
  }
  throw new Error("orderk CLI not found. Set ORDERK_BIN, install the native binary on PATH, or let the npm package download its vendor binary.");
}

function resolvePackageLocalBinary(): string | undefined {
  const rawEntrypoint = process.argv[1] ? path.resolve(process.argv[1]) : process.cwd();
  const entrypoint = safeRealpath(rawEntrypoint);
  const packageRoot = path.resolve(path.dirname(entrypoint), "..");
  const packageVendor = path.join(packageRoot, "vendor", binaryFilename());
  return fs.existsSync(packageVendor) ? packageVendor : undefined;
}

function resolvePathBinary(): string | undefined {
  const currentEntrypoint = process.argv[1] ? safeRealpath(path.resolve(process.argv[1])) : "";
  const names = process.platform === "win32" ? ["orderk.exe", "orderk.cmd", "orderk.bat", "orderk"] : ["orderk"];
  for (const dir of (process.env.PATH || "").split(path.delimiter)) {
    if (!dir) continue;
    for (const name of names) {
      const candidate = path.join(dir, name);
      if (!fs.existsSync(candidate)) continue;
      if (currentEntrypoint && safeRealpath(candidate) === currentEntrypoint) continue;
      return candidate;
    }
  }
  return undefined;
}

function binaryFilename(): string {
  return process.platform === "win32" ? "orderk.exe" : "orderk";
}

function safeRealpath(target: string): string {
  try {
    return fs.realpathSync(target);
  } catch {
    return target;
  }
}
