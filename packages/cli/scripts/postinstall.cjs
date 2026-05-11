
const fs = require("node:fs");
const path = require("node:path");

const pkg = require("../package.json");

main().catch((error) => {
  console.warn(`[orderk] postinstall binary setup failed: ${error?.message || error}`);
});

async function main() {
  const target = vendorBinaryPath();
  if (fs.existsSync(target)) {
    return;
  }

  if (process.env.ORDERK_SKIP_BINARY_DOWNLOAD === "1") {
    console.warn("[orderk] skipped binary download because ORDERK_SKIP_BINARY_DOWNLOAD=1");
    return;
  }

  const url = resolveBinaryUrl();
  if (!url) {
    console.warn("[orderk] no release binary is available for this platform; set ORDERK_BIN or build orderk locally");
    return;
  }

  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`download failed with HTTP ${response.status}`);
  }

  const bytes = Buffer.from(await response.arrayBuffer());
  fs.mkdirSync(path.dirname(target), { recursive: true });
  fs.writeFileSync(target, bytes, { mode: 0o755 });
  fs.chmodSync(target, 0o755);
  console.log(`[orderk] downloaded native binary from ${url}`);
}

function vendorBinaryPath() {
  return path.join(__dirname, "..", "vendor", binaryFilename());
}

function binaryFilename() {
  return process.platform === "win32" ? "orderk.exe" : "orderk";
}

function resolveBinaryUrl() {
  const explicit = process.env.ORDERK_BINARY_URL;
  if (explicit) return explicit;

  const platform = platformName();
  const arch = process.arch;
  if (!platform || !arch) return "";

  // Current release pipeline ships Linux x64 first; the mapping below is future-proof.
  const supported = platform === "linux" && arch === "x64";
  if (!supported) return "";

  const version = String(pkg.version || "0.1.0");
  const repoUrl = repositoryUrl();
  const { owner, repo } = parseRepo(repoUrl);
  const asset = `orderk-v${version}-${platform}-${arch}`;
  return `https://github.com/${owner}/${repo}/releases/download/v${version}/${asset}`;
}

function platformName() {
  if (process.platform === "linux") return "linux";
  if (process.platform === "darwin") return "darwin";
  if (process.platform === "win32") return "windows";
  return "";
}

function repositoryUrl() {
  const repo = pkg.repository;
  if (repo && typeof repo === "object" && typeof repo.url === "string") return repo.url;
  if (typeof repo === "string") return repo;
  return process.env.ORDERK_RELEASE_REPOSITORY || "https://github.com/bsbofmusic/orderk.git";
}

function parseRepo(url) {
  const normalized = String(url).replace(/\.git$/, "");
  const match = normalized.match(/github\.com[:/](.+?)\/(.+)$/);
  if (!match) {
    return { owner: "bsbofmusic", repo: "orderk" };
  }
  return { owner: match[1], repo: match[2] };
}
