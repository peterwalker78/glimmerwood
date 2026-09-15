// Home. The core hands this page everything it shows through
// `window.wispHome.show`; the page never asks for anything. Its buttons are
// plain links to glimmerwood://home/do/..., which the core catches.

import type { HomeData } from "../protocol.gen.js";
import { showDiary } from "./diary.js";
import { bank, drawGarden, H, partOfClock, W } from "./garden.js";

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

  drawGarden(garden, data.part, data.plants, data.seed);
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

window.wispHome = { show, reveal };

if (window.wispHomeData) {
  show(window.wispHomeData);
} else {
  // Until the core's data arrives, a scene for the local hour.
  const part = partOfClock(new Date().getHours());
  document.body.dataset["part"] = part;
  drawGarden(garden, part, [], 1);
}
