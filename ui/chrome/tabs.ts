// The tabs, down the column: each site's icon, or a quiet letter when it has
// none, the title beside it once the column is wide enough, and a small mark
// on tabs making sound that mutes them.
//
// Tiles are kept and updated in place, never rebuilt: a tile replaced between
// a press and its release would swallow the click.

import type { TabInfo, ToCore } from "../protocol.gen.js";

interface Tile {
  item: HTMLDivElement;
  badge: HTMLElement;
  icon: string | undefined;
  letter: string;
  title: HTMLSpanElement;
  sound: HTMLButtonElement;
}

export class TabList {
  private tiles = new Map<number, Tile>();
  private order: number[] = [];
  private selected = 0;

  constructor(
    private readonly strip: HTMLElement,
    private readonly send: (message: ToCore) => void,
  ) {
    strip.addEventListener("keydown", (event) => this.key(event));
  }

  render(tabs: TabInfo[], selected: number, icons: Map<number, string>): void {
    this.selected = selected;
    const keep = new Set(tabs.map((tab) => tab.id));
    for (const [id, tile] of this.tiles) {
      if (!keep.has(id)) {
        tile.item.remove();
        this.tiles.delete(id);
      }
    }
    let previous: Element | null = null;
    for (const tab of tabs) {
      const tile = this.tiles.get(tab.id) ?? this.create(tab.id);
      this.update(tile, tab, tab.id === selected, icons.get(tab.id));
      const expected: Element | null = previous ? previous.nextElementSibling : this.strip.firstElementChild;
      if (expected !== tile.item) {
        if (previous) previous.after(tile.item);
        else this.strip.prepend(tile.item);
      }
      previous = tile.item;
    }
    this.order = tabs.map((tab) => tab.id);
  }

  private create(id: number): Tile {
    const item = document.createElement("div");
    item.className = "tab";
    item.setAttribute("role", "tab");

    const title = document.createElement("span");
    title.className = "title";

    const sound = document.createElement("button");
    sound.className = "sound";
    sound.type = "button";
    sound.tabIndex = -1;
    sound.addEventListener("mousedown", (event) => event.stopPropagation());
    sound.addEventListener("click", (event) => {
      event.stopPropagation();
      this.send({ type: "toggle_mute", id });
    });

    const close = document.createElement("button");
    close.className = "close";
    close.type = "button";
    close.tabIndex = -1;
    close.title = "Close (Ctrl+W)";
    close.textContent = "×";
    close.addEventListener("mousedown", (event) => event.stopPropagation());
    close.addEventListener("click", (event) => {
      event.stopPropagation();
      this.send({ type: "close_tab", id });
    });

    // Select on press, as native tab strips do.
    item.addEventListener("mousedown", (event) => {
      if (event.button === 0) this.send({ type: "select_tab", id });
    });
    item.addEventListener("auxclick", (event) => {
      if (event.button === 1) this.send({ type: "close_tab", id });
    });

    const badge = document.createElement("span");
    item.append(badge, title, sound, close);
    const tile: Tile = { item, badge, icon: undefined, letter: "", title, sound };
    this.tiles.set(id, tile);
    return tile;
  }

  private update(tile: Tile, tab: TabInfo, selected: boolean, icon: string | undefined): void {
    const name = tab.title || "New tab";
    const { item } = tile;
    if (item.title !== name) {
      item.title = name;
      item.setAttribute("aria-label", name);
      tile.title.textContent = name;
      const close = item.querySelector(".close");
      close?.setAttribute("aria-label", `Close ${name}`);
    }
    item.setAttribute("aria-selected", String(selected));
    item.tabIndex = selected ? 0 : -1;
    item.classList.toggle("loading", tab.loading);

    const letter = letterFor(tab);
    if (icon !== tile.icon || (!icon && letter !== tile.letter)) {
      let badge: HTMLElement;
      if (icon) {
        const image = document.createElement("img");
        image.src = icon;
        image.alt = "";
        badge = image;
      } else {
        badge = document.createElement("span");
        badge.textContent = letter;
      }
      badge.className = "icon";
      badge.setAttribute("aria-hidden", "true");
      tile.badge.replaceWith(badge);
      tile.badge = badge;
      tile.icon = icon;
      tile.letter = letter;
    }

    const sound = tile.sound;
    sound.hidden = tab.sound === "silent";
    sound.classList.toggle("muted", tab.sound === "muted");
    if (sound.dataset["state"] !== tab.sound) {
      sound.dataset["state"] = tab.sound;
      sound.replaceChildren(speaker(tab.sound === "muted"));
    }
    const action = tab.sound === "muted" ? "Unmute" : "Mute";
    sound.title = `${action} this tab`;
    sound.setAttribute("aria-label", `${action} ${name}`);
  }

  // Arrow keys move between tabs, as in any tab list; Delete closes.
  private key(event: KeyboardEvent): void {
    const index = this.order.indexOf(this.selected);
    if (index < 0) return;
    let next: number | undefined;
    switch (event.key) {
      case "ArrowDown":
        next = this.order[Math.min(this.order.length - 1, index + 1)];
        break;
      case "ArrowUp":
        next = this.order[Math.max(0, index - 1)];
        break;
      case "Home":
        next = this.order[0];
        break;
      case "End":
        next = this.order[this.order.length - 1];
        break;
      case "Delete":
        this.send({ type: "close_tab", id: this.selected });
        event.preventDefault();
        return;
      default:
        return;
    }
    event.preventDefault();
    if (next !== undefined && next !== this.selected) this.send({ type: "select_tab", id: next });
  }

  focusSelected(): void {
    this.tiles.get(this.selected)?.item.focus();
  }
}

// A small speaker, with sound waves or struck through.
function speaker(muted: boolean): SVGSVGElement {
  const ns = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(ns, "svg");
  svg.setAttribute("viewBox", "0 0 16 16");
  svg.setAttribute("aria-hidden", "true");
  const path = document.createElementNS(ns, "path");
  path.setAttribute(
    "d",
    muted ? "M2.5 6h2.5l3.5-3v10l-3.5-3h-2.5zM11 6l4 4m0-4-4 4" : "M2.5 6h2.5l3.5-3v10l-3.5-3h-2.5zM11 5.5a3.5 3.5 0 0 1 0 5M12.8 3.5a6 6 0 0 1 0 9",
  );
  svg.append(path);
  return svg;
}

// The first letter of the site, without "www.": a blank tab gets none.
function letterFor(tab: TabInfo): string {
  const host = tab.host.replace(/^www\./, "");
  return (host.charAt(0) || "").toUpperCase();
}
