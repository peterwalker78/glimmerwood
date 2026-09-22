// Home. The core hands this page everything it shows through
// `window.wispHome.show`; the page never asks for anything. Its buttons are
// plain links to glimmerwood://home/do/..., which the core catches.

import type { Download, HomeData, Page, Progress } from "../protocol.gen.js";
import { showDiary } from "./diary.js";
import { bank, drawGarden, H, partOfClock, W } from "./garden.js";
import { letter, placeCard, span } from "./place.js";

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
const thisWeek = element("this-week", HTMLElement);
const days = element("days", HTMLDivElement);
const arriving = element("downloads", HTMLElement);
const downloads = element("download-list", HTMLUListElement);
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

  places.replaceChildren(...data.places.map(placeCard));

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

  showPages(data.pages);
  showDownloads(data.downloads);

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

// --- This week ----------------------------------------------------------------

const DAY_MS = 86_400_000;

function midnight(at: number): number {
  const date = new Date(at);
  return new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
}

// "Today", "Yesterday", then the weekday. Nothing here is a week old, so the
// weekday alone is never ambiguous.
function dayName(at: number, now: number): string {
  const between = Math.round((midnight(now) - midnight(at)) / DAY_MS);
  if (between <= 0) return "Today";
  if (between === 1) return "Yesterday";
  return new Date(at).toLocaleDateString(undefined, { weekday: "long" });
}

function visit(page: Page): HTMLLIElement {
  const item = document.createElement("li");
  item.className = "visit";
  const link = document.createElement("a");
  link.href = page.url;
  link.title = page.url;
  link.append(letter(page.title || page.host), span("title", page.title || page.url), span("host", page.host));
  item.append(link);
  return item;
}

// A week of pages at once is a wall. Each day is folded away behind its own
// name, today's open, and a heavy day shows this many before it waits to be
// asked for the rest — enough that a quiet day is never cut, and few enough
// that a heavy one can't run away down the page.
const FIRST_FEW = 12;

// Which days have been opened and which closed, against a default of today
// open and the rest away, and which have been asked for in full. The page is
// redrawn whenever a page is remembered, and it should come back as it was
// left.
const opened = new Set<number>();
const closed = new Set<number>();
const unfolded = new Set<number>();

function count(pages: number): string {
  return pages === 1 ? "1 page" : `${pages} pages`;
}

function dayGroup(start: number, name: string, pages: Page[], today: boolean): HTMLDetailsElement {
  const group = document.createElement("details");
  group.className = "day-group";
  group.open = today ? !closed.has(start) : opened.has(start);
  group.addEventListener("toggle", () => {
    const remember = group.open ? opened : closed;
    const forget = group.open ? closed : opened;
    remember.add(start);
    forget.delete(start);
  });

  const summary = document.createElement("summary");
  const heading = document.createElement("h3");
  heading.className = "day-name";
  heading.append(span("name", name), span("day-count", count(pages.length)));
  summary.append(heading);

  const list = document.createElement("ul");
  list.className = "visits";
  list.append(...pages.map(visit));

  // The rest of a long day waits behind a word rather than a scroll.
  const rest = pages.length - FIRST_FEW;
  if (rest > 0 && !unfolded.has(start)) {
    const hidden = Array.from(list.children).slice(FIRST_FEW) as HTMLLIElement[];
    for (const item of hidden) item.hidden = true;
    const more = document.createElement("li");
    more.className = "more";
    const button = document.createElement("button");
    button.type = "button";
    button.className = "more-link";
    button.textContent = rest === 1 ? "Show the last one" : `Show the other ${rest}`;
    button.addEventListener("click", () => {
      unfolded.add(start);
      for (const item of hidden) item.hidden = false;
      more.remove();
    });
    more.append(button);
    list.append(more);
  }

  group.append(summary, list);
  return group;
}

// The pages arrive newest first, so a day is a run of neighbours.
function showPages(pages: Page[]): void {
  thisWeek.hidden = pages.length === 0;
  const now = Date.now();
  const today = midnight(now);
  const groups: HTMLDetailsElement[] = [];
  let start: number | null = null;
  let day: Page[] = [];
  const close = (): void => {
    if (start !== null && day.length > 0) groups.push(dayGroup(start, dayName(day[0]!.at, now), day, start === today));
  };
  for (const page of pages) {
    const at = midnight(page.at);
    if (at !== start) {
      close();
      start = at;
      day = [];
    }
    day.push(page);
  }
  close();
  days.replaceChildren(...groups);
}

// --- Downloads ------------------------------------------------------------------

const PROGRESS_WORD: Record<Progress, string> = {
  running: "Arriving",
  saved: "Saved",
  stopped: "Stopped",
  failed: "Didn't finish",
};

function download(file: Download): HTMLLIElement {
  const item = document.createElement("li");
  item.className = "download";
  const name = span("name", file.name);
  name.title = file.path;
  item.append(name);
  // How far along, while it is still coming and the size is known at all.
  if (file.progress === "running" && file.fraction !== null) {
    const track = document.createElement("span");
    track.className = "filling";
    const filled = document.createElement("i");
    filled.style.width = `${Math.round(Math.min(1, Math.max(0, file.fraction)) * 100)}%`;
    track.append(filled);
    item.append(track);
  }
  item.append(span("state", PROGRESS_WORD[file.progress]));
  if (file.progress === "saved") {
    const show = document.createElement("a");
    show.className = "show";
    show.href = `glimmerwood://home/do/open-download/${file.id}`;
    show.setAttribute("aria-label", `Show ${file.name}`);
    show.textContent = "Show";
    item.append(show);
  }
  return item;
}

function showDownloads(files: Download[]): void {
  arriving.hidden = files.length === 0;
  downloads.replaceChildren(...files.map(download));
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

// The forget link points at whichever stretch the drop-down is showing.
{
  const choice = document.getElementById("forget-since");
  const link = document.getElementById("forget");
  if (choice instanceof HTMLSelectElement && link instanceof HTMLAnchorElement) {
    const point = (): void => {
      link.href = `glimmerwood://home/do/forget-since/${choice.value}`;
    };
    choice.addEventListener("change", point);
    point();
  }
}
