
import { spawn } from "node:child_process";
import { resolveOrderkBinary } from "./resolveBinary.js";

export function spawnOrderk(args: string[], binaryPath?: string) {
  const bin = resolveOrderkBinary(binaryPath);
  return spawn(bin, args, { stdio: "pipe" });
}

export function runOrderk(args: string[], binaryPath?: string): Promise<void> {
  return new Promise((resolve, reject) => {
    const child = spawnOrderk(args, binaryPath);
    let stderr = "";
    child.stderr.on("data", (chunk) => (stderr += chunk.toString()));
    child.stdout.on("data", (chunk) => process.stdout.write(chunk));
    child.on("error", reject);
    child.on("close", (code) => {
      if (code === 0) return resolve();
      reject(new Error(stderr.trim() || `orderk exited with code ${code}`));
    });
  });
}
