// The wisp's days on Home: today's line, the last seven days, what
// moved it this week, and the weeks before. Sizes are only compared with each
// other; there are no minutes, scores or percentages anywhere in it.

import type { DiaryDay, DiarySite, DiaryWeek, HomeWisp, PlantKind, TimeKind, TimeShare, WispPhase } from "../protocol.gen.js";

const SVG = "http://www.w3.org/2000/svg";

function node<K extends keyof SVGElementTagNameMap>(
  name: K,
  attributes: Record<string, string | number>,
): SVGElementTagNameMap[K] {
  const el = document.createElementNS(SVG, name);
  for (const [key, value] of Object.entries(attributes)) el.setAttribute(key, String(value));
  return el;
}

function html<K extends keyof HTMLElementTagNameMap>(name: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const el = document.createElement(name);
  if (className) el.className = className;
  if (text !== undefined) el.textContent = text;
  return el;
}

const PHASE_WORD: Record<WispPhase, string> = {
  rested: "bright",
  engaged: "busy",
  clouded: "clouded",
  drained: "sleepy",
};

const KIND_WORD: Record<TimeKind, string> = {
  restoring: "restoring",
  everyday: "everyday",
  wearing: "wearing",
};

const PLANT_WORD: Record<PlantKind, string> = {
  moss: "moss",
  fern: "a fern",
  flower: "a flower",
};

// --- Today ---------------------------------------------------------------------

const W = 720;
const LEFT = 62;
const RIGHT = 8;
const TOP = 16;
const PLOT = 112;
const H = TOP + PLOT + 24;

function x(minute: number): number {
  return LEFT + (Math.min(minute, 1440) / 1440) * (W - LEFT - RIGHT);
}

function y(dose: number): number {
  return TOP + Math.max(0, Math.min(1, dose)) * PLOT;
}

// "6am", "noon", "6pm", "midnight".
function clock(hour: number): string {
  if (hour === 0) return "midnight";
  if (hour === 12) return "noon";
  return hour < 12 ? `${hour}am` : `${hour - 12}pm`;
}

function phaseOf(wisp: HomeWisp, dose: number): WispPhase {
  if (dose >= wisp.drained) return "drained";
  if (dose >= wisp.clouded) return "clouded";
  if (dose >= wisp.engaged) return "engaged";
  return "rested";
}

function drawToday(svg: SVGSVGElement, wisp: HomeWisp): void {
  const layer: SVGElement[] = [];
  const bands: [WispPhase, number, number][] = [
    ["rested", 0, wisp.engaged],
    ["engaged", wisp.engaged, wisp.clouded],
    ["clouded", wisp.clouded, wisp.drained],
    ["drained", wisp.drained, 1],
  ];
  bands.forEach(([phase, from, to], i) => {
    layer.push(node("rect", { class: `band band-${i % 2}`, x: LEFT, y: y(from), width: W - LEFT - RIGHT, height: y(to) - y(from) }));
    const label = node("text", { class: "band-label", x: LEFT - 10, y: (y(from) + y(to)) / 2 + 4, "text-anchor": "end" });
    const word = PHASE_WORD[phase];
    label.textContent = word.charAt(0).toUpperCase() + word.slice(1);
    layer.push(label);
  });

  // The night window, and the part of the day still to come.
  layer.push(node("rect", { class: "night", x: x(wisp.night_from), y: TOP, width: x(wisp.night_until) - x(wisp.night_from), height: PLOT }));
  const night = node("text", { class: "night-label", x: x(wisp.night_from) + 6, y: TOP - 5 });
  night.textContent = "night";
  layer.push(night);
  layer.push(node("rect", { class: "later", x: x(wisp.now_minute), y: TOP, width: Math.max(0, x(1440) - x(wisp.now_minute)), height: PLOT }));

  for (let hour = 0; hour < 24; hour += 6) {
    const minute = (hour * 60 - wisp.day_start_minute + 1440) % 1440;
    layer.push(node("line", { class: "tick", x1: x(minute), x2: x(minute), y1: TOP + PLOT, y2: TOP + PLOT + 4 }));
    const label = node("text", { class: "tick-label", x: x(minute), y: TOP + PLOT + 17, "text-anchor": "middle" });
    label.textContent = clock(hour);
    layer.push(label);
  }

  // The line: solid where the wisp was watching, dotted across time the
  // browser was closed.
  let solid = "";
  let dotted = "";
  wisp.today.forEach((point, i) => {
    const px = x(point.minute).toFixed(1);
    const py = y(point.dose).toFixed(1);
    const previous = wisp.today[i - 1];
    if (!previous) {
      solid += `M${px} ${py}`;
    } else if (point.gap) {
      dotted += `M${x(previous.minute).toFixed(1)} ${y(previous.dose).toFixed(1)}L${px} ${py}`;
      solid += `M${px} ${py}`;
    } else {
      solid += `L${px} ${py}`;
    }
  });
  if (dotted) layer.push(node("path", { class: "line gap", d: dotted }));
  if (solid) layer.push(node("path", { class: "line", d: solid }));

  const last = wisp.today[wisp.today.length - 1];
  if (last) {
    const phase = phaseOf(wisp, last.dose);
    layer.push(node("circle", { class: `now-glow phase-${phase}`, cx: x(last.minute), cy: y(last.dose), r: 9 }));
    layer.push(node("circle", { class: `now phase-${phase}`, cx: x(last.minute), cy: y(last.dose), r: 3.5 }));
  }

  svg.setAttribute("viewBox", `0 0 ${W} ${H}`);
  svg.setAttribute("aria-label", todayInWords(wisp));
  svg.replaceChildren(...layer);
}

// For screen readers: where the line spent most of the day, and where it is.
function todayInWords(wisp: HomeWisp): string {
  const counts = new Map<WispPhase, number>();
  for (const point of wisp.today) {
    const phase = phaseOf(wisp, point.dose);
    counts.set(phase, (counts.get(phase) ?? 0) + 1);
  }
  const most = [...counts.entries()].sort((a, b) => b[1] - a[1])[0];
  const last = wisp.today[wisp.today.length - 1];
  const now = last ? PHASE_WORD[phaseOf(wisp, last.dose)] : "bright";
  return most && wisp.today.length > 1
    ? `Today the wisp has been mostly ${PHASE_WORD[most[0]]}, and is ${now} now.`
    : `The wisp is ${now} now.`;
}

// --- Days and weeks ------------------------------------------------------------

function stack(shares: TimeShare[], className: string): HTMLDivElement {
  const bar = html("div", className);
  for (const share of shares) {
    const part = html("span", `part ${share.kind}`);
    part.style.flexGrow = String(share.share);
    bar.append(part);
  }
  return bar;
}

function mostly(shares: TimeShare[]): string {
  const top = [...shares].sort((a, b) => b.share - a.share)[0];
  return top ? `mostly ${KIND_WORD[top.kind]}` : "time away";
}

function plantMark(kind: PlantKind): SVGSVGElement {
  const svg = node("svg", { class: `plant ${kind}`, viewBox: "0 0 16 16", "aria-hidden": "true" });
  switch (kind) {
    case "moss":
      svg.append(node("circle", { cx: 5, cy: 11.5, r: 3 }), node("circle", { cx: 9.5, cy: 10.5, r: 3.6 }), node("circle", { cx: 12.5, cy: 12, r: 2.4 }));
      break;
    case "fern":
      svg.append(node("path", { d: "M8 15C8 10 7 6 4.5 2M8 11 5 9.5M7.6 8 4.8 6.2M8 11l3-1.8M7.8 7.6l2.6-2" }));
      break;
    case "flower":
      svg.append(node("path", { class: "stem", d: "M8 15V8" }));
      for (let i = 0; i < 5; i++) {
        const a = (i / 5) * Math.PI * 2 - Math.PI / 2;
        svg.append(node("circle", { class: "petal", cx: 8 + Math.cos(a) * 2.8, cy: 5.5 + Math.sin(a) * 2.8, r: 2 }));
      }
      svg.append(node("circle", { class: "heart", cx: 8, cy: 5.5, r: 1.4 }));
      break;
  }
  return svg;
}

function dayColumn(day: DiaryDay): HTMLLIElement {
  const item = html("li", day.known ? "day" : "day unknown");
  const words = [day.label === "Today" ? "Today" : day.label];
  if (day.known) {
    words.push(day.size > 0 ? mostly(day.shares) : "time away");
    if (day.peak) words.push(`at its heaviest ${PHASE_WORD[day.peak]}`);
    if (day.plant) words.push(`grew ${PLANT_WORD[day.plant]}`);
  } else {
    words.push("before the wisp's time");
  }
  item.setAttribute("aria-label", words.join(", "));
  item.title = words.slice(1).join(", ");

  const well = html("div", "well");
  if (day.known && day.size > 0) {
    const bar = stack(day.shares, "stack");
    bar.style.height = `${Math.max(3, day.size * 100)}%`;
    well.append(bar);
  }
  const peak = html("span", day.peak ? `peak phase-${day.peak}` : "peak none");
  const label = html("span", "day-label", day.label);
  const plant = html("span", "day-plant");
  if (day.plant) plant.append(plantMark(day.plant));
  item.append(well, peak, label, plant);
  item.querySelectorAll(":scope > *").forEach((child) => child.setAttribute("aria-hidden", "true"));
  return item;
}

function weekRow(week: DiaryWeek): HTMLLIElement {
  const item = html("li", "week-row");
  const growth = week.plants.length > 0 ? `, grew ${week.plants.map((p) => PLANT_WORD[p]).join(", ")}` : "";
  item.setAttribute("aria-label", `${week.label}: ${week.size > 0 ? mostly(week.shares) : "time away"}${growth}`);
  const track = html("div", "track");
  if (week.size > 0) {
    const bar = stack(week.shares, "stack across");
    bar.style.width = `${Math.max(1, week.size * 100)}%`;
    track.append(bar);
  }
  const plants = html("span", "week-plants");
  for (const plant of week.plants) plants.append(plantMark(plant));
  item.append(html("span", "week-label", week.label), track, plants);
  item.querySelectorAll(":scope > *").forEach((child) => child.setAttribute("aria-hidden", "true"));
  return item;
}

function siteRow(site: DiarySite): HTMLLIElement {
  const item = html("li", "site");
  const meter = html("span", "bars");
  meter.setAttribute("aria-hidden", "true");
  for (let i = 0; i < 3; i++) meter.append(html("i", i < site.bars ? "on" : ""));
  item.append(html("span", "site-label", site.label), meter);
  return item;
}

// --- Putting it on the page ------------------------------------------------------

export type DiaryElements = {
  today: SVGSVGElement;
  week: HTMLOListElement;
  moved: HTMLElement;
  restoring: HTMLUListElement;
  wearing: HTMLUListElement;
  earlierWrap: HTMLElement;
  earlier: HTMLOListElement;
};

export function showDiary(els: DiaryElements, wisp: HomeWisp): void {
  drawToday(els.today, wisp);
  els.week.replaceChildren(...wisp.week.map(dayColumn));

  const restoring = wisp.sites.filter((s) => s.kind === "restoring");
  const wearing = wisp.sites.filter((s) => s.kind === "wearing");
  els.restoring.replaceChildren(...restoring.map(siteRow));
  els.wearing.replaceChildren(...wearing.map(siteRow));
  els.restoring.parentElement?.toggleAttribute("hidden", restoring.length === 0);
  els.wearing.parentElement?.toggleAttribute("hidden", wearing.length === 0);
  els.moved.hidden = wisp.sites.length === 0;

  els.earlier.replaceChildren(...wisp.earlier.map(weekRow));
  els.earlierWrap.hidden = wisp.earlier.length === 0;
}
