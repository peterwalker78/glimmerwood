// The hover caption: how the wisp is, what is moving it now,
// what moved it over the last 15 minutes, and the other tabs. Words and small
// bars, never numbers.

import type { CaptionLine, ToChrome } from "../protocol.gen.js";

type Wisp = Extract<ToChrome, { type: "wisp" }>;

// How the wisp is and which way it's heading, in weather rather than
// verdicts. Drained reads sleepy, never sad.
function mood(wisp: Wisp): string {
  if (wisp.night) return "Winding down";
  const state = { rested: "Bright", engaged: "Busy", clouded: "Clouded", drained: "Sleepy" }[wisp.phase];
  switch (wisp.trend) {
    case "rising":
      return `${state}, getting heavier`;
    case "falling":
      return wisp.phase === "rested" ? state : `${state}, recovering`;
    case "steady":
      return state;
  }
}

function words(line: CaptionLine): { label: string; effect: string } {
  const label = line.heard ? `${line.label}, playing` : line.label;
  switch (line.kind) {
    case "wearing":
      return { label, effect: line.heard ? "wearing, softly" : "wearing" };
    case "restoring":
      return { label, effect: line.heard ? "soothing" : "restoring" };
    case "ordinary_sites":
      return { label: line.label || "ordinary sites", effect: "resting" };
    case "private":
      return { label: "giving you some privacy", effect: "" };
    case "holding":
      return { label: line.label || "this page", effect: line.label ? "not on my lists yet" : "holding steady" };
    case "playing":
      return { label, effect: "" };
    case "away":
      return { label: "away", effect: "resting" };
    default: {
      const unhandled: never = line.kind;
      throw new Error(`unhandled caption line ${String(unhandled)}`);
    }
  }
}

function row(label: string, effect: string, bars: number | null): HTMLLIElement {
  const item = document.createElement("li");
  const name = document.createElement("span");
  name.className = "label";
  name.textContent = label;
  const meter = document.createElement("span");
  meter.className = "bars";
  meter.setAttribute("aria-hidden", "true");
  for (let i = 0; i < 3; i++) {
    const bar = document.createElement("i");
    if (bars !== null && i < bars) bar.className = "on";
    meter.append(bar);
  }
  if (bars === null || bars === 0) meter.classList.add("none");
  const what = document.createElement("span");
  what.className = "effect";
  what.textContent = effect;
  item.append(name, meter, what);
  return item;
}

function lineRow(line: CaptionLine, bars: boolean): HTMLLIElement {
  const { label, effect } = words(line);
  return row(label, effect, bars ? line.bars : null);
}

function heading(text: string): HTMLLIElement {
  const item = document.createElement("li");
  item.className = "heading";
  item.textContent = text;
  return item;
}

function note(text: string, className: string): HTMLLIElement {
  const item = document.createElement("li");
  item.className = className;
  item.textContent = text;
  return item;
}

function plural(n: number, one: string, many: string): string {
  return n === 1 ? `1 ${one}` : `${n} ${many}`;
}

// "Now" first, so a change of site shows at once.
export function renderCaption(list: HTMLElement, wisp: Wisp): void {
  list.replaceChildren();
  list.append(heading("Now"), note(mood(wisp), "mood"));
  for (const line of wisp.now) list.append(lineRow(line, false));

  list.append(heading("Last 15 minutes"));
  if (wisp.caption.length === 0) list.append(note("Steady", "steady"));
  for (const line of wisp.caption) list.append(lineRow(line, true));

  if (wisp.quiet_tabs + wisp.untouched_tabs > 0) {
    list.append(heading("Other tabs"));
    if (wisp.quiet_tabs > 0) {
      list.append(row(plural(wisp.quiet_tabs, "quiet tab", "quiet tabs"), "no weight", null));
    }
    if (wisp.untouched_tabs > 0) {
      const label = `${wisp.untouched_tabs} untouched since yesterday`;
      list.append(row(label, "slowing rest", null));
    }
  }
  list.append(note("Click to see its days", "hint"));
}
