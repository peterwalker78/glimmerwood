// Settings. The core hands this page everything it shows through
// `window.wispSettings.show`; the page never asks for anything. Every control
// follows a link to glimmerwood://settings/do/..., which the core catches and
// answers by handing the page its data again.

import type { NewsFeed, NewsKind, Rating, SettingsData, SiteRating, TimeChoice } from "../protocol.gen.js";
import { WORDS, setSlider, slider } from "./rating.js";

declare global {
  interface Window {
    wispSettings: { show(data: SettingsData): void };
    // Left by the core in case it arrives before this script has run.
    wispSettingsData?: SettingsData;
  }
}

function element<T extends HTMLElement>(id: string, kind: new () => T): T {
  const found = document.getElementById(id);
  if (!(found instanceof kind)) throw new Error(`settings page has no #${id}`);
  return found;
}

const lookupForm = element("lookup-form", HTMLFormElement);
const lookup = element("lookup", HTMLInputElement);
const lookupFailed = element("lookup-failed", HTMLParagraphElement);
const lookupResult = element("lookup-result", HTMLUListElement);
const yours = element("yours", HTMLUListElement);
const noRatings = element("no-ratings", HTMLParagraphElement);
const problem = element("ratings-problem", HTMLParagraphElement);
const file = element("ratings-file", HTMLElement);
const ask = element("ask", HTMLInputElement);
const nightStarts = element("night-starts", HTMLSelectElement);
const nightEnds = element("night-ends", HTMLSelectElement);
const news = element("news", HTMLDivElement);
const newsActions = element("news-actions", HTMLDivElement);
const newsSend = element("news-send", HTMLButtonElement);
const newsRemove = element("news-remove", HTMLButtonElement);
const newsWhere = element("news-where", HTMLParagraphElement);
const newsDone = element("news-done", HTMLParagraphElement);
const newsMissing = element("news-missing", HTMLParagraphElement);

const DO = "glimmerwood://settings/do/";

function follow(action: string): void {
  location.href = DO + action;
}

function rateLink(site: string, rating: Rating | "glimmerwood"): string {
  return `${DO}rate/${rating}/${encodeURIComponent(site)}`;
}

function text(className: string, words: string): HTMLSpanElement {
  const span = document.createElement("span");
  span.className = className;
  span.textContent = words;
  return span;
}

function link(href: string, words: string, pressed?: boolean): HTMLAnchorElement {
  const a = document.createElement("a");
  a.href = href;
  a.textContent = words;
  if (pressed !== undefined) {
    a.setAttribute("role", "button");
    a.setAttribute("aria-pressed", String(pressed));
  }
  return a;
}

// What stands behind a site's rating, in a few words.
function whose(site: SiteRating): string {
  if (site.yours !== null) return "Your rating";
  if (site.matched === site.site) return "Glimmerwood's rating";
  if (site.matched) return `Counted as part of ${site.matched}`;
  return "On none of the lists, so it holds the wisp steady";
}

// One site: its name, how it counts, the slider, and the other choices.
// Rows are rebuilt only when something about them changed, so a slider being
// dragged isn't replaced under the pointer.
function row(site: SiteRating): HTMLLIElement {
  const item = document.createElement("li");
  item.className = "rating";
  item.dataset["key"] = JSON.stringify(site);

  const head = document.createElement("div");
  head.className = "rating-head";
  head.append(text("site", site.site), text("whose", whose(site)));

  const scale = slider(`How ${site.site} leaves you`, site.rating, (rating) => {
    if (rating !== site.rating || site.yours === null) location.href = rateLink(site.site, rating);
  });

  const others = document.createElement("div");
  others.className = "others";
  for (const rating of ["news", "private", "unrated"] as const) {
    const choice = link(rateLink(site.site, rating), WORDS[rating], site.rating === rating);
    others.append(choice);
  }
  if (site.yours !== null) {
    const back = site.seed === null
      ? link(rateLink(site.site, "glimmerwood"), "Forget your rating")
      : link(rateLink(site.site, "glimmerwood"), `Use Glimmerwood's: ${WORDS[site.seed]}`);
    back.className = "back";
    others.append(back);
  }
  setSlider(scale, site.rating);
  item.append(head, scale, others);
  return item;
}

function rows(list: HTMLUListElement, sites: SiteRating[]): void {
  const keys = [...list.children].map((child) => (child as HTMLElement).dataset["key"]);
  if (keys.length === sites.length && sites.every((site, i) => keys[i] === JSON.stringify(site))) return;
  list.replaceChildren(...sites.map(row));
}

function options(select: HTMLSelectElement, choices: TimeChoice[], chosen: string): void {
  if (select.options.length !== choices.length) {
    select.replaceChildren(
      ...choices.map((choice) => {
        const option = document.createElement("option");
        option.value = choice.value;
        option.textContent = choice.label;
        return option;
      }),
    );
  }
  select.value = chosen;
}

const KINDS: [NewsKind, string][] = [
  ["working", "What's working"],
  ["light", "Light"],
  ["awe", "Awe"],
];

// The feeds by kind, each marked once Newsboat has it.
function feeds(list: NewsFeed[], ready: boolean): void {
  const key = JSON.stringify([list, ready]);
  if (news.dataset["key"] === key) return;
  news.dataset["key"] = key;
  news.replaceChildren(
    ...KINDS.map(([kind, heading]) => {
      const group = document.createElement("div");
      group.className = "news-kind";
      const items = document.createElement("ul");
      for (const feed of list.filter((f) => f.kind === kind)) {
        const item = document.createElement("li");
        item.append(text("name", feed.name), text("site", feed.site.split("/")[0] ?? feed.site));
        if (ready && feed.added) item.append(text("in", "In Newsboat"));
        items.append(item);
      }
      group.append(text("kind", heading), items);
      return group;
    }),
  );
}

function showNews(data: SettingsData): void {
  const ready = data.newsboat_file !== "";
  const missing = data.good_news.filter((feed) => !feed.added).length;
  feeds(data.good_news, ready);
  newsActions.hidden = !ready;
  newsMissing.hidden = ready;
  newsSend.hidden = missing === 0;
  newsSend.textContent = missing === data.good_news.length
    ? "Send happy news to Newsboat"
    : `Send the ${missing} it hasn't got`;
  newsRemove.hidden = missing === data.good_news.length;
  newsWhere.hidden = !ready;
  newsWhere.replaceChildren(
    "They go at the end of ",
    Object.assign(document.createElement("code"), { textContent: data.newsboat_file }),
    ", tagged glimmerwood, and only those lines ever come out again. Nothing is fetched until Newsboat fetches it.",
  );
  newsDone.textContent = data.newsboat_done;
}

function show(data: SettingsData): void {
  rows(lookupResult, data.lookup);
  lookupFailed.hidden = !data.lookup_failed;
  lookupFailed.textContent = data.lookup_failed ? `"${data.lookup_failed}" isn't a site Glimmerwood can rate. Try something like example.org.` : "";

  rows(yours, data.ratings);
  noRatings.hidden = data.ratings.length > 0;
  problem.hidden = !data.ratings_problem;
  problem.textContent = data.ratings_problem
    ? `Your file has a mistake in it, so Glimmerwood is using its own ratings until it's fixed: ${data.ratings_problem}`
    : "";
  file.textContent = data.ratings_file;

  ask.checked = data.ask;
  options(nightStarts, data.night_start_choices, data.night_starts);
  options(nightEnds, data.night_end_choices, data.night_ends);
  showNews(data);
}

lookupForm.addEventListener("submit", (event) => {
  event.preventDefault();
  follow(`look-up/${encodeURIComponent(lookup.value.trim())}`);
});
ask.addEventListener("change", () => follow(`ask/${ask.checked ? "on" : "off"}`));
for (const select of [nightStarts, nightEnds]) {
  select.addEventListener("change", () =>
    follow(`night/${encodeURIComponent(nightStarts.value)}/${encodeURIComponent(nightEnds.value)}`),
  );
}
newsSend.addEventListener("click", () => follow("newsboat/add"));
newsRemove.addEventListener("click", () => follow("newsboat/remove"));

window.wispSettings = { show };
if (window.wispSettingsData) show(window.wispSettingsData);
