
import { mkdir, copyFile, rm, writeFile } from "node:fs/promises";
import path from "node:path";
const out = path.resolve("dist");
await rm(out, { recursive: true, force: true });
await mkdir(out, { recursive: true });
for (const file of ["main.js", "manifest.json", "styles.css", "versions.json"]) {
  await copyFile(file, path.join(out, file));
}
await writeFile(path.join(out, "README.txt"), "orderk Obsidian plugin package\n");
