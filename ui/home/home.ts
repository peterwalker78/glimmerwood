// Home. The core hands this page everything it shows through
// `window.wispHome.show`; the page never asks for anything. Its buttons are
// plain links to glimmerwood://home/do/..., which the core catches.

import type { DayPart, HomeData, HomePlant } from "../protocol.gen.js";
import { showDiary } from "./diary.js";

declare global {
  interface Window {
    wispHome: { show(data: HomeData): void; reveal(): void };
    // Left by the core in case it arrives before this script has run.
    wispHomeData?: HomeData;
  }
}

function element<T extends HTMLElement>(id: string, kind: new () => T): T {
  const found = document.getElementById(id);
  if (!(found instanceof kind)) throw new Error(`home page has no #${id}`);
  return found;
}

const title = element("title", HTMLHeadingElement);
const line = element("line", HTMLParagraphElement);
const scene = element("scene", HTMLElement);
const about = element("about", HTMLDivElement);
const aboutTitle = element("about-title", HTMLHeadingElement);
const aboutText = element("about-text", HTMLDivElement);
const aboutDone = element("about-done", HTMLAnchorElement);
const aboutWisp = document.getElementById("about-wisp") as unknown as SVGSVGElement;
const bubble = about.querySelector(".bubble") as HTMLElement;
const explain = element("explain", HTMLElement);
const explainText = element("explain-text", HTMLParagraphElement);
const explainDone = element("explain-done", HTMLAnchorElement);
const places = element("places", HTMLUListElement);
const bookmarks = element("bookmarks", HTMLUListElement);
const noBookmarks = element("no-bookmarks", HTMLParagraphElement);
const garden = document.getElementById("garden") as unknown as SVGSVGElement;
const diary = {
  section: element("wisp", HTMLElement),
  today: document.getElementById("today") as unknown as SVGSVGElement,
  week: element("week", HTMLOListElement),
  moved: element("moved", HTMLDivElement),
  restoring: element("moved-restoring", HTMLUListElement),
  wearing: element("moved-wearing", HTMLUListElement),
  earlierWrap: element("earlier-wrap", HTMLDivElement),
  earlier: element("earlier", HTMLOListElement),
};

const SVG = "http://www.w3.org/2000/svg";

function letter(text: string): HTMLSpanElement {
  const tile = document.createElement("span");
  tile.className = "tile";
  tile.setAttribute("aria-hidden", "true");
  tile.textContent = (text.replace(/^www\./, "").trim().charAt(0) || "·").toUpperCase();
  return tile;
}

function span(className: string, text: string): HTMLSpanElement {
  const s = document.createElement("span");
  s.className = className;
  s.textContent = text;
  return s;
}

function show(data: HomeData): void {
  document.body.dataset["part"] = data.part;
  title.textContent = data.title;
  line.textContent = data.line;
  document.title = "Home";

  about.hidden = !data.about;
  scene.classList.toggle("has-about", Boolean(data.about));
  if (data.about) {
    aboutTitle.textContent = data.about.title;
    aboutText.replaceChildren(
      ...data.about.paragraphs.map((text) => {
        const p = document.createElement("p");
        p.textContent = text;
        return p;
      }),
    );
    aboutDone.textContent = data.about.done;
    aboutDone.href = "glimmerwood://home/do/got-it/about";
    seatWisp();
  }

  explain.hidden = !data.explain;
  if (data.explain) {
    explainText.textContent = data.explain.text;
    explainDone.href = `glimmerwood://home/do/got-it/${data.explain.topic}`;
  }

  places.replaceChildren(
    ...data.places.map((place) => {
      const item = document.createElement("li");
      item.className = "place";
      const link = document.createElement("a");
      link.href = place.url;
      link.append(letter(place.name), span("name", place.name), span("line", place.line));
      if (place.yours) link.append(span("yours", "You come back here"));
      item.append(link);
      return item;
    }),
  );

  noBookmarks.hidden = data.bookmarks.length > 0;
  bookmarks.replaceChildren(
    ...data.bookmarks.map((bookmark) => {
      const item = document.createElement("li");
      item.className = "bookmark";
      const link = document.createElement("a");
      link.href = bookmark.url;
      link.append(letter(bookmark.title || bookmark.host), span("title", bookmark.title || bookmark.url), span("host", bookmark.host));
      const forget = document.createElement("a");
      forget.className = "forget";
      forget.href = `glimmerwood://home/do/forget-bookmark/${bookmark.id}`;
      forget.title = "Remove this bookmark";
      forget.setAttribute("aria-label", `Remove ${bookmark.title || bookmark.url}`);
      forget.textContent = "×";
      item.append(link, forget);
      return item;
    }),
  );

  drawGarden(data.part, data.plants, data.seed);
  showDiary(diary, data.wisp);

  // Opened by clicking the wisp: once the history is drawn, go to it.
  if (!shown && location.hash === "#wisp") reveal();
  shown = true;
}
let shown = false;

function reveal(): void {
  const still = matchMedia("(prefers-reduced-motion: reduce)").matches;
  diary.section.scrollIntoView({ block: "start", behavior: still ? "auto" : "smooth" });
}

// --- The garden ----------------------------------------------------------------

// A small seeded generator, so each install's garden keeps its own shape.
function random(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function node<K extends keyof SVGElementTagNameMap>(
  name: K,
  attributes: Record<string, string | number>,
): SVGElementTagNameMap[K] {
  const el = document.createElementNS(SVG, name);
  for (const [key, value] of Object.entries(attributes)) el.setAttribute(key, String(value));
  return el;
}

const W = 1200;
const H = 360;

// Sit the introducing wisp on the near bank. The garden is scaled to cover
// the scene and anchored to its bottom middle, so where the bank meets the
// wisp depends on the scene's size.
let seated = 0;
function seatWisp(): void {
  if (about.hidden) return;
  const box = scene.getBoundingClientRect();
  const wisp = aboutWisp.getBoundingClientRect();
  if (box.width === 0 || wisp.height === 0) return;
  const scale = Math.max(box.width / W, box.height / H);
  const gardenX = W / 2 + (wisp.left + wisp.width / 2 - box.left - box.width / 2) / scale;
  const bankY = box.bottom - (H - bank(gardenX) - 3) * scale;
  // The moss under the wisp is 90/96 of the way down its drawing.
  const naturalTop = wisp.top - seated;
  seated = Math.round(bankY - wisp.height * (90 / 96) - naturalTop);
  aboutWisp.style.transform = `translateY(${seated}px)`;
  // Point the bubble's tail at the wisp's face.
  const room = bubble.getBoundingClientRect();
  const face = naturalTop + seated + wisp.height * 0.62;
  const tail = Math.min(room.height - 40, Math.max(18, room.bottom - face - 9));
  about.style.setProperty("--tail", `${Math.round(tail)}px`);
}
new ResizeObserver(seatWisp).observe(scene);
// It settles in with the rest of the page; measure again once it has.
about.addEventListener("animationend", seatWisp);

// The height of the near bank at x.
function bank(x: number): number {
  return 292 - 26 * Math.sin((x / W) * Math.PI * 1.1 + 0.4) - 10 * Math.sin((x / W) * Math.PI * 3.3);
}

function hill(base: number, amplitude: number, phase: number, fill: string): SVGPathElement {
  let d = `M0 ${H}`;
  for (let x = 0; x <= W; x += 40) {
    const y = base - amplitude * Math.sin((x / W) * Math.PI * 1.6 + phase) - amplitude * 0.35 * Math.sin((x / W) * Math.PI * 4.1 + phase * 2);
    d += ` L${x} ${y.toFixed(1)}`;
  }
  return node("path", { d: `${d} L${W} ${H} Z`, fill });
}

function drawGarden(part: DayPart, plants: HomePlant[], seed: number): void {
  const rand = random(seed || 1);
  const style = getComputedStyle(document.body);
  const colour = (name: string) => style.getPropertyValue(name).trim();
  const layer: SVGElement[] = [];

  // Sun or moon, and stars after dark.
  if (part === "night") {
    const stars = random(seed ^ 0x9e3779b9);
    for (let i = 0; i < 70; i++) {
      layer.push(node("circle", { cx: stars() * W, cy: stars() * 210, r: stars() * 1.2 + 0.3, fill: "#f4f1e6", opacity: 0.35 + stars() * 0.5 }));
    }
    layer.push(node("circle", { cx: 930, cy: 86, r: 22, fill: "#f2ecd8" }));
    layer.push(node("circle", { cx: 941, cy: 80, r: 20, fill: colour("--sky-top"), opacity: 0.92 }));
  } else {
    const [cx, cy] = part === "morning" ? [960, 170] : part === "day" ? [880, 70] : [760, 214];
    const evening = part === "evening";
    const glow = node("radialGradient", { id: "sun-glow" });
    glow.append(
      node("stop", { offset: "0%", "stop-color": evening ? "#fbe0b8" : "#fffbef", "stop-opacity": 0.9 }),
      node("stop", { offset: "35%", "stop-color": evening ? "#f7c58f" : "#fff5dc", "stop-opacity": 0.45 }),
      node("stop", { offset: "100%", "stop-color": evening ? "#f7c58f" : "#fff5dc", "stop-opacity": 0 }),
    );
    const defs = node("defs", {});
    defs.append(glow);
    layer.push(
      defs,
      node("circle", { cx, cy, r: 130, fill: "url(#sun-glow)" }),
      node("circle", { cx, cy, r: 28, fill: evening ? "#fadcb2" : "#fffaf0" }),
    );
  }

  layer.push(hill(236, 26, 0.8, colour("--far-hill")));
  layer.push(hill(262, 20, 2.3, colour("--near-hill")));

  let bankPath = `M0 ${H}`;
  for (let x = 0; x <= W; x += 20) bankPath += ` L${x} ${bank(x).toFixed(1)}`;
  layer.push(node("path", { d: `${bankPath} L${W} ${H} Z`, fill: colour("--ground") }));

  // Moss cushions along the bank, always there: the garden's floor.
  for (let i = 0; i < 26; i++) {
    const x = rand() * W;
    layer.push(node("ellipse", { cx: x, cy: bank(x) + 4, rx: 14 + rand() * 26, ry: 5 + rand() * 6, fill: colour("--ground-shade"), opacity: 0.7 }));
  }

  // One plant per day that grew, placed by the seed, newest nearest the
  // middle so recent days are easy to see. Nothing is ever taken away; a
  // long history just grows denser.
  const shown = plants.slice(-90);
  shown.forEach((plant, i) => {
    const spread = (i + 1) / (shown.length + 1);
    const x = 40 + (W - 80) * ((spread + rand() * 0.6) % 1);
    const y = bank(x) + 3 + rand() * 20;
    const size = 0.8 + rand() * 0.5;
    layer.push(...drawPlant(plant.kind, x, y, size, rand));
  });

  // Fireflies from calm nights, out in the evening and at night.
  if (part === "evening" || part === "night") {
    const calm = shown.filter((p) => p.fireflies).length;
    for (let i = 0; i < Math.min(calm, 24); i++) {
      const x = rand() * W;
      const y = bank(x) - 20 - rand() * 70;
      layer.push(node("circle", { cx: x, cy: y, r: 7, fill: "#f6e7a1", opacity: 0.18 }));
      layer.push(node("circle", { cx: x, cy: y, r: 1.8, fill: "#fff6c9" }));
    }
  }

  garden.replaceChildren(...layer);
}

function drawPlant(kind: HomePlant["kind"], x: number, y: number, size: number, rand: () => number): SVGElement[] {
  const green = getComputedStyle(document.body).getPropertyValue("--near-hill").trim();
  switch (kind) {
    case "moss": {
      const out: SVGElement[] = [];
      for (let i = 0; i < 4; i++) {
        out.push(node("circle", { cx: x + (rand() - 0.5) * 18 * size, cy: y - rand() * 5, r: (4 + rand() * 4) * size, fill: "#7fa05f", opacity: 0.9 }));
      }
      return out;
    }
    case "fern": {
      const out: SVGElement[] = [];
      const fronds = 3 + Math.floor(rand() * 3);
      for (let i = 0; i < fronds; i++) {
        const lean = (i - (fronds - 1) / 2) * 12 + (rand() - 0.5) * 6;
        const h = (34 + rand() * 18) * size;
        out.push(node("path", {
          d: `M${x} ${y} Q${x + lean * 0.4} ${y - h * 0.6} ${x + lean} ${y - h}`,
          stroke: "#5f8c4e",
          "stroke-width": 2.2 * size,
          fill: "none",
          "stroke-linecap": "round",
        }));
        for (let j = 1; j < 5; j++) {
          const t = j / 5;
          const px = x + lean * t * t;
          const py = y - h * t;
          out.push(node("ellipse", { cx: px - 4 * size, cy: py, rx: 4 * size, ry: 1.8 * size, fill: "#6f9b5b", transform: `rotate(-25 ${px} ${py})` }));
          out.push(node("ellipse", { cx: px + 4 * size, cy: py, rx: 4 * size, ry: 1.8 * size, fill: "#6f9b5b", transform: `rotate(25 ${px} ${py})` }));
        }
      }
      return out;
    }
    case "flower": {
      const petals = ["#c9b3e3", "#f0d88f", "#eec2cf", "#f5efe0", "#b9d3e6"];
      const petal = petals[Math.floor(rand() * petals.length)] ?? "#f5efe0";
      const h = (22 + rand() * 20) * size;
      const top = y - h;
      const out: SVGElement[] = [
        node("path", { d: `M${x} ${y} Q${x + 3} ${y - h / 2} ${x} ${top}`, stroke: green || "#6a8f59", "stroke-width": 1.8, fill: "none" }),
      ];
      for (let i = 0; i < 5; i++) {
        const a = (i / 5) * Math.PI * 2;
        out.push(node("circle", { cx: x + Math.cos(a) * 4.2 * size, cy: top + Math.sin(a) * 4.2 * size, r: 3.6 * size, fill: petal }));
      }
      out.push(node("circle", { cx: x, cy: top, r: 2.4 * size, fill: "#e0a94f" }));
      return out;
    }
    default: {
      const unhandled: never = kind;
      throw new Error(`unknown plant ${String(unhandled)}`);
    }
  }
}

window.wispHome = { show, reveal };

if (window.wispHomeData) {
  show(window.wispHomeData);
} else {
  // Until the core's data arrives, a scene for the local hour.
  const hour = new Date().getHours();
  const part: DayPart = hour >= 23 || hour < 5 ? "night" : hour >= 18 ? "evening" : hour >= 12 ? "day" : "morning";
  document.body.dataset["part"] = part;
  drawGarden(part, [], 1);
}
