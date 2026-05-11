import { App, Modal, Notice } from "obsidian";
import { OrderkClient } from "./orderkClient";
import { OrderkSearchResult } from "./types";

export class OrderkSearchModal extends Modal {
  private results: OrderkSearchResult[] = [];
  private inputEl!: HTMLInputElement;
  private resultsEl!: HTMLDivElement;

  constructor(app: App, private client: OrderkClient) {
    super(app);
  }

  onOpen() {
    const { contentEl } = this;
    contentEl.empty();
    contentEl.createEl("h2", { text: "orderk search" });

    this.inputEl = contentEl.createEl("input", {
      attr: { type: "search", placeholder: "Search vault..." },
    });
    this.inputEl.addEventListener("input", () => void this.refresh());

    this.resultsEl = contentEl.createDiv({ cls: "orderk-results" });
    void this.refresh();
  }

  onClose() {
    this.contentEl.empty();
  }

  private async refresh() {
    const query = this.inputEl?.value?.trim();
    if (!query) {
      this.results = [];
      this.renderResults();
      return;
    }
    try {
      this.results = await this.client.search(query);
      this.renderResults();
    } catch (error) {
      new Notice(String(error));
    }
  }

  private renderResults() {
    this.resultsEl.empty();
    for (const item of this.results) {
      const row = this.resultsEl.createDiv({ cls: "orderk-search-result" });
      row.createEl("div", { text: item.title ?? item.path, cls: "orderk-search-title" });
      row.createEl("small", { text: item.path, cls: "orderk-search-path" });
      if (item.snippet) row.createEl("div", { text: item.snippet, cls: "orderk-search-snippet" });
      row.addEventListener("click", () => new Notice(`Selected ${item.path}`));
    }
  }
}
