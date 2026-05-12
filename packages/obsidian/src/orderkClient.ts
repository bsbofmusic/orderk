import { App } from "obsidian";
import { spawn } from "node:child_process";
import * as fs from "node:fs";
import * as path from "node:path";
import {
  OrderkEvalResponse,
  OrderkFeedbackEvent,
  OrderkHealthReport,
  OrderkSearchResult,
  OrderkSearchResponse,
  OrderkStatusResponse,
} from "./types";

export type OrderkSettings = {
  binaryPath?: string;
  vaultPath?: string;
  indexOnStartup: boolean;
  searchLimit: number;
  openInNewPane: boolean;
  debugLogging: boolean;
  embeddingProvider: string;
  embeddingModel: string;
  embeddingDim: number;
};

export const DEFAULT_SETTINGS: OrderkSettings = {
  indexOnStartup: false,
  searchLimit: 10,
  openInNewPane: false,
  debugLogging: false,
  embeddingProvider: "siliconflow",
  embeddingModel: "BAAI/bge-m3",
  embeddingDim: 1024,
};

export class OrderkClient {
  constructor(private app: App, private settings: OrderkSettings) {}

  async rebuildIndex(): Promise<void> {
    await this.run([
      "index",
      "--vault",
      this.vaultPath(),
      "--db",
      this.dbPath(),
      "--embedding-provider",
      this.settings.embeddingProvider,
      "--embedding-dim",
      String(this.settings.embeddingDim),
      "--embedding-model",
      this.settings.embeddingModel,
      "--json",
    ]);
  }

  async searchResponse(query: string): Promise<OrderkSearchResponse> {
    return this.parseJson(
      await this.run([
        "search",
        "--db",
        this.dbPath(),
        "--query",
        query,
        "--limit",
        String(this.settings.searchLimit),
        "--embedding-provider",
        this.settings.embeddingProvider,
        "--embedding-dim",
        String(this.settings.embeddingDim),
        "--embedding-model",
        this.settings.embeddingModel,
        "--json",
      ]),
    );
  }

  async search(query: string): Promise<OrderkSearchResult[]> {
    const response = await this.searchResponse(query);
    return response.results ?? [];
  }

  async status(): Promise<OrderkStatusResponse> {
    return this.parseJson(
      await this.run([
        "status",
        "--db",
        this.dbPath(),
        "--json",
      ]),
    );
  }

  async health(): Promise<OrderkHealthReport> {
    return this.parseJson(
      await this.run([
        "health",
        "--db",
        this.dbPath(),
        "--vault",
        this.vaultPath(),
        "--embedding-provider",
        this.settings.embeddingProvider,
        "--embedding-dim",
        String(this.settings.embeddingDim),
        "--embedding-model",
        this.settings.embeddingModel,
        "--json",
      ]),
    );
  }

  async doctor(): Promise<OrderkHealthReport> {
    return this.parseJson(
      await this.run([
        "doctor",
        "--db",
        this.dbPath(),
        "--vault",
        this.vaultPath(),
        "--embedding-provider",
        this.settings.embeddingProvider,
        "--embedding-dim",
        String(this.settings.embeddingDim),
        "--embedding-model",
        this.settings.embeddingModel,
        "--json",
      ]),
    );
  }

  async eval(queriesPath: string): Promise<OrderkEvalResponse> {
    return this.parseJson(
      await this.run([
        "eval",
        "--db",
        this.dbPath(),
        "--queries",
        queriesPath,
        "--limit",
        String(this.settings.searchLimit),
        "--embedding-provider",
        this.settings.embeddingProvider,
        "--embedding-dim",
        String(this.settings.embeddingDim),
        "--embedding-model",
        this.settings.embeddingModel,
        "--json",
      ]),
    );
  }

  async sendFeedback(event: OrderkFeedbackEvent): Promise<void> {
    await this.run(["feedback", "--db", this.dbPath(), "--event", JSON.stringify(event), "--json"]);
  }

  async version(): Promise<string> {
    return await this.run(["--version"]);
  }

  private vaultPath(): string {
    const value = this.settings.vaultPath?.trim();
    if (!value) throw new Error("Set vault path in orderk settings");
    return value;
  }

  private dbPath(): string {
    return `${this.vaultPath()}/.obsidian/orderk/orderk.sqlite`;
  }

  private async run(args: string[]): Promise<string> {
    const bin = resolveOrderkBinary(this.settings.binaryPath);
    return await new Promise((resolve, reject) => {
      const child = spawn(bin, args, { stdio: ["ignore", "pipe", "pipe"] });
      let stdout = "";
      let stderr = "";
      child.stdout.on("data", (d) => (stdout += d.toString()));
      child.stderr.on("data", (d) => (stderr += d.toString()));
      child.on("error", reject);
      child.on("close", (code) => {
        if (code === 0) return resolve(stdout.trim());
        const message = stderr.trim();
        try {
          const parsed = JSON.parse(message) as { message?: string };
          reject(new Error(parsed.message ?? (message || `orderk exited ${code}`)));
          return;
        } catch {
          reject(new Error(message || `orderk exited ${code}`));
        }
      });
    });
  }

  private parseJson<T>(raw: string): T {
    return JSON.parse(raw) as T;
  }
}

function resolveOrderkBinary(explicitPath?: string): string {
  const candidates = [explicitPath, process.env.ORDERK_BIN, localBinary(), "orderk"].filter(Boolean) as string[];
  for (const candidate of candidates) {
    if (candidate === "orderk") return candidate;
    if (fs.existsSync(candidate)) return candidate;
  }
  throw new Error("orderk CLI not found. Set ORDERK_BIN, configure the plugin binary path, or install orderk on PATH.");
}

function localBinary(): string | undefined {
  const packageRoot = path.resolve(__dirname);
  const candidates = [
    path.join(packageRoot, "vendor", process.platform === "win32" ? "orderk.exe" : "orderk"),
  ];
  return candidates.find((candidate) => fs.existsSync(candidate));
}
