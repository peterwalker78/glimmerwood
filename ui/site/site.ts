// The welcome page. There is no core here and nothing to remember: it draws
// the garden for the hour, sits the wisp on the bank asleep, and offers a
// few good places. Nothing is stored and nothing is sent.

import { bank, drawGarden, H, partOfClock, W } from "./garden.js";
import { placeCard } from "./place.js";
import { PLACES, ROTATE_HOURS, type SitePlace } from "./places.gen.js";

const SHOWN = 6;
// At most this many of any one kind, so the six are a mixture.
const PER_KIND = 2;

const scene = document.getElementById("scene") as HTMLElement;
const garden = document.getElementById("garden") as unknown as SVGSVGElement;
const dozing = document.querySelector(".dozing") as SVGSVGElement;
const places = document.getElementById("places") as HTMLUListElement;

// Somewhere between 0 and 1 for a place in a stretch, stable while the
// stretch lasts and unrelated to the last one.
function unit(url: string, stretch: number): number {
  let a = (stretch * 0x9e3779b1) >>> 0;
  for (let i = 0; i < url.length; i++) a = (Math.imul(a ^ url.charCodeAt(i), 0x5bd1e995) + 0x7ed55d16) >>> 0;
  a = Math.imul(a ^ (a >>> 15), a | 1) >>> 0;
  return ((a ^ (a >>> 14)) >>> 0) / 4294967296;
}

// The picks Home makes lean on the wisp's phase and on where the user
// already goes. Neither is known here, so this is simply: what suits the
// hour and the month, in an order that changes with the stretch.
function choose(now: Date): SitePlace[] {
  const part = partOfClock(now.getHours());
  const month = now.getMonth() + 1;
  const stretch = Math.floor(now.getTime() / 1000 / (ROTATE_HOURS * 3600));
  const uk = navigator.language.toUpperCase().endsWith("-GB");

  const fits = PLACES.filter(
    (p) =>
      p.moments.includes(part) &&
      (p.seasons.length === 0 || p.seasons.includes(month)) &&
      (!p.uk || uk),
  );
  const order = fits
    .map((p) => ({ p, score: unit(p.url, stretch) + (p.fresh ? 0.12 : 0) }))
    .sort((a, b) => b.score - a.score)
    .map((s) => s.p);

  const chosen: SitePlace[] = [];
  const kinds = new Map<string, number>();
  for (const place of order) {
    if (chosen.length === SHOWN) break;
    const seen = kinds.get(place.kind) ?? 0;
    if (seen >= PER_KIND) continue;
    kinds.set(place.kind, seen + 1);
    chosen.push(place);
  }
  // A quiet hour with little to offer would rather repeat a kind than be short.
  for (const place of order) {
    if (chosen.length === SHOWN) break;
    if (!chosen.includes(place)) chosen.push(place);
  }
  return chosen;
}

// Sit the wisp on the near bank. The garden is scaled to cover the scene and
// anchored to its bottom middle, so where the bank meets the wisp depends on
// the scene's size.
let seated = 0;
function seat(): void {
  const box = scene.getBoundingClientRect();
  const wisp = dozing.getBoundingClientRect();
  if (box.width === 0 || wisp.height === 0) return;
  const scale = Math.max(box.width / W, box.height / H);
  const gardenX = W / 2 + (wisp.left + wisp.width / 2 - box.left - box.width / 2) / scale;
  const bankY = box.bottom - (H - bank(gardenX) - 3) * scale;
  // The moss under the wisp is 90/96 of the way down its drawing.
  const naturalTop = wisp.top - seated;
  seated = Math.round(bankY - wisp.height * (90 / 96) - naturalTop);
  dozing.style.transform = `translateY(${seated}px)`;
}

const now = new Date();
document.body.dataset["part"] = partOfClock(now.getHours());
drawGarden(garden, partOfClock(now.getHours()), [], 1);
places.replaceChildren(...choose(now).map(placeCard));
seat();
new ResizeObserver(seat).observe(scene);
