
import { App, Plugin, PluginSettingTab, Setting, Notice } from "obsidian";
import { OrderkClient, OrderkSettings, DEFAULT_SETTINGS } from "./orderkClient";
import { OrderkSearchModal } from "./searchModal";

export default class OrderkPlugin extends Plugin {
  settings: OrderkSettings = DEFAULT_SETTINGS;
  client!: OrderkClient;

  async onload() {
    await this.loadSettings();
    this.client = new OrderkClient(this.app, this.settings);
    this.addSettingTab(new OrderkSettingTab(this.app, this));
    if (this.settings.indexOnStartup) {
      this.app.workspace.onLayoutReady(() => {
        void this.client.rebuildIndex()
          .then(() => new Notice("orderk startup index completed"))
          .catch((error) => {
            const message = error instanceof Error ? error.message : String(error);
            new Notice(`orderk startup index failed: ${message}`);
          });
      });
    }
    this.addCommand({
      id: "orderk-rebuild-index",
      name: "Orderk: Rebuild Index",
      callback: async () => {
        await this.client.rebuildIndex();
        new Notice("orderk index rebuilt");
      },
    });
    this.addCommand({
      id: "orderk-search",
      name: "Orderk: Search",
      callback: () => new OrderkSearchModal(this.app, this.client).open(),
    });
    this.addCommand({
      id: "orderk-health",
      name: "Orderk: Health Check",
      callback: async () => {
        const report = await this.client.health();
        new Notice(`orderk health: ${report.state}`);
      },
    });
    this.addCommand({
      id: "orderk-doctor",
      name: "Orderk: Doctor",
      callback: async () => {
        const report = await this.client.doctor();
        new Notice(`orderk doctor: ${report.state}`);
      },
    });
  }

  onunload() {}

  async loadSettings() {
    this.settings = Object.assign({}, DEFAULT_SETTINGS, await this.loadData());
  }

  async saveSettings() {
    await this.saveData(this.settings);
  }
}

class OrderkSettingTab extends PluginSettingTab {
  plugin: OrderkPlugin;
  constructor(app: App, plugin: OrderkPlugin) {
    super(app, plugin);
    this.plugin = plugin;
  }
  display(): void {
    const { containerEl } = this;
    containerEl.empty();
    containerEl.createEl("h2", { text: "orderk settings" });
    new Setting(containerEl)
      .setName("Vault path")
      .setDesc("Path to your Obsidian vault")
      .addText((text) => text.setValue(this.plugin.settings.vaultPath ?? "").onChange(async (value) => {
        this.plugin.settings.vaultPath = value;
        await this.plugin.saveSettings();
      }));
    new Setting(containerEl)
      .setName("CLI binary path")
      .addText((text) => text.setValue(this.plugin.settings.binaryPath ?? "").onChange(async (value) => {
        this.plugin.settings.binaryPath = value;
        await this.plugin.saveSettings();
      }));
    new Setting(containerEl)
      .setName("Embedding provider")
      .setDesc("Use siliconflow for production cloud vectors; use mock only for offline tests.")
      .addText((text) => text.setValue(this.plugin.settings.embeddingProvider ?? "siliconflow").onChange(async (value) => {
        this.plugin.settings.embeddingProvider = value || "siliconflow";
        await this.plugin.saveSettings();
      }));
    new Setting(containerEl)
      .setName("Embedding model")
      .setDesc("Production default: BAAI/bge-m3. API key must be available to Obsidian through HERMES_SILICONFLOW_API_KEY or SILICONFLOW_API_KEY.")
      .addText((text) => text.setValue(this.plugin.settings.embeddingModel ?? "BAAI/bge-m3").onChange(async (value) => {
        this.plugin.settings.embeddingModel = value || "BAAI/bge-m3";
        await this.plugin.saveSettings();
      }));
    new Setting(containerEl)
      .setName("Embedding dimension")
      .setDesc("BAAI/bge-m3 uses 1024 dimensions in the default orderk profile.")
      .addText((text) => text.setValue(String(this.plugin.settings.embeddingDim ?? 1024)).onChange(async (value) => {
        this.plugin.settings.embeddingDim = Number(value) || 1024;
        await this.plugin.saveSettings();
      }));
    new Setting(containerEl)
      .setName("Index on startup")
      .setDesc("Run one incremental index pass when Obsidian finishes loading. No background watcher or polling is started.")
      .addToggle((toggle) => toggle.setValue(this.plugin.settings.indexOnStartup).onChange(async (value) => {
        this.plugin.settings.indexOnStartup = value;
        await this.plugin.saveSettings();
      }));
    new Setting(containerEl)
      .setName("Search limit")
      .addText((text) => text.setValue(String(this.plugin.settings.searchLimit)).onChange(async (value) => {
        this.plugin.settings.searchLimit = Number(value) || 10;
        await this.plugin.saveSettings();
      }));
  }
}
