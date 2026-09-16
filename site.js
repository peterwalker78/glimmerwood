// The welcome page. There is no core here and nothing to remember: it draws
// the garden for the hour, sits the wisp on the bank asleep, and offers a
// few good places. Nothing is stored and nothing is sent.
import { bank, drawGarden, H, partOfClock, W } from "./garden.js";
import { placeCard } from "./place.js";
import { FACTS } from "./facts.gen.js";
import { PLACES, ROTATE_HOURS } from "./places.gen.js";
const SHOWN = 6;
// At most this many of any one kind, so the six are a mixture.
const PER_KIND = 2;
const scene = document.getElementById("scene");
const motes = document.getElementById("motes");
const wake = document.getElementById("wake");
const caughtSoFar = document.getElementById("caught");
const factText = document.getElementById("fact-text");
const title = document.getElementById("greeting-title");
const line = document.getElementById("greeting-line");
const factSource = document.getElementById("fact-source");
const garden = document.getElementById("garden");
const dozing = document.querySelector(".dozing");
const places = document.getElementById("places");
// How far down the list of places we have walked. Asking for more shows the
// next six rather than shuffling: a list to look through, not a wheel to
// spin again.
let walked = 0;
// Somewhere between 0 and 1 for a place in a stretch, stable while the
// stretch lasts and unrelated to the last one.
function unit(url, stretch) {
    let a = (stretch * 0x9e3779b1) >>> 0;
    for (let i = 0; i < url.length; i++)
        a = (Math.imul(a ^ url.charCodeAt(i), 0x5bd1e995) + 0x7ed55d16) >>> 0;
    a = Math.imul(a ^ (a >>> 15), a | 1) >>> 0;
    return ((a ^ (a >>> 14)) >>> 0) / 4294967296;
}
// The picks Home makes lean on the wisp's phase and on where the user
// already goes. Neither is known here, so this is simply: what suits the
// hour and the month, in an order that changes with the stretch.
function choose(now, from = 0) {
    const part = partOfClock(now.getHours());
    const month = now.getMonth() + 1;
    const stretch = Math.floor(now.getTime() / 1000 / (ROTATE_HOURS * 3600));
    const uk = navigator.language.toUpperCase().endsWith("-GB");
    const fits = PLACES.filter((p) => p.moments.includes(part) &&
        (p.seasons.length === 0 || p.seasons.includes(month)) &&
        (!p.uk || uk));
    const order = fits
        .map((p) => ({ p, score: unit(p.url, stretch) + (p.fresh ? 0.12 : 0) }))
        .sort((a, b) => b.score - a.score)
        .map((s) => s.p);
    // Wrap round rather than run out: the pool is long, but not endless.
    const start = order.length > 0 ? from % order.length : 0;
    const walkable = [...order.slice(start), ...order.slice(0, start)];
    const chosen = [];
    const kinds = new Map();
    for (const place of walkable) {
        if (chosen.length === SHOWN)
            break;
        const seen = kinds.get(place.kind) ?? 0;
        if (seen >= PER_KIND)
            continue;
        kinds.set(place.kind, seen + 1);
        chosen.push(place);
    }
    // A quiet hour with little to offer would rather repeat a kind than be short.
    for (const place of walkable) {
        if (chosen.length === SHOWN)
            break;
        if (!chosen.includes(place))
            chosen.push(place);
    }
    return chosen;
}
// Sit the wisp on the near bank. The garden is scaled to cover the scene and
// anchored to its bottom middle, so where the bank meets the wisp depends on
// the scene's size.
let seated = 0;
function seat() {
    const box = scene.getBoundingClientRect();
    const wisp = dozing.getBoundingClientRect();
    if (box.width === 0 || wisp.height === 0)
        return;
    const scale = Math.max(box.width / W, box.height / H);
    const gardenX = W / 2 + (wisp.left + wisp.width / 2 - box.left - box.width / 2) / scale;
    const bankY = box.bottom - (H - bank(gardenX) - 3) * scale;
    // The moss under the wisp is 90/96 of the way down its drawing.
    const naturalTop = wisp.top - seated;
    seated = Math.round(bankY - wisp.height * (90 / 96) - naturalTop);
    dozing.style.transform = `translateY(${seated}px)`;
}
// --- One thing worth knowing ----------------------------------------------
let lastFact = -1;
function showFact() {
    if (FACTS.length === 0)
        return;
    let next = Math.floor(Math.random() * FACTS.length);
    // Never the same one twice running, so "another" always is one.
    if (FACTS.length > 1 && next === lastFact)
        next = (next + 1) % FACTS.length;
    lastFact = next;
    const fact = FACTS[next];
    if (!fact)
        return;
    factText.textContent = fact.text;
    factSource.textContent = fact.source;
    factSource.href = fact.url;
}
// --- Catch ------------------------------------------------------------------
/// How many throws there are, and how long a mote is in the air. It is meant
/// to be gentle: nothing is scored, nothing is kept, and it ends by itself.
const THROWS = 5;
const FLIGHT_MS = 2400;
/// What the page says when the wisp is asleep, and when it is not.
const ASLEEP = {
    title: "The wisp is dozing",
    line: "It can\u2019t see anything from here \u2014 no history, nothing to watch. This is only a page. But it left the light on, and somewhere to go.",
};
const AWAKE = {
    title: "The wisp is awake",
    line: "It throws a light for you to catch. Five throws, and then it goes back to sleep.",
};
function greet(words) {
    title.textContent = words.title;
    line.textContent = words.line;
}
let playing = false;
let caught = 0;
let thrown = 0;
function wispAt() {
    const box = scene.getBoundingClientRect();
    const wisp = dozing.getBoundingClientRect();
    return {
        x: wisp.left - box.left + wisp.width / 2,
        y: wisp.top - box.top + wisp.height * 0.45,
    };
}
function throwOne() {
    if (!playing)
        return;
    if (thrown >= THROWS) {
        finish();
        return;
    }
    thrown += 1;
    const box = scene.getBoundingClientRect();
    const from = wispAt();
    // Somewhere off to one side, never off the edge of the scene.
    const side = Math.random() < 0.5 ? -1 : 1;
    const reach = Math.min(box.width * 0.34, 260);
    const to = {
        x: Math.min(Math.max(from.x + side * (reach * (0.5 + Math.random() * 0.5)), 40), box.width - 40),
        y: from.y + 10,
    };
    const rise = Math.min(from.y - 20, 70 + Math.random() * 40);
    const mote = document.createElement("button");
    mote.className = "mote";
    mote.type = "button";
    mote.setAttribute("aria-label", "Catch the light");
    motes.append(mote);
    let done = false;
    const land = (got) => {
        if (done)
            return;
        done = true;
        if (got) {
            caught += 1;
            mote.classList.add("caught");
            say();
            setTimeout(() => mote.remove(), 400);
        }
        else {
            mote.remove();
        }
        setTimeout(throwOne, got ? 500 : 300);
    };
    mote.addEventListener("click", () => land(true));
    const started = performance.now();
    const step = (at) => {
        if (!playing) {
            mote.remove();
            return;
        }
        const along = Math.min((at - started) / FLIGHT_MS, 1);
        mote.style.left = `${from.x + (to.x - from.x) * along}px`;
        // Up and down again: a throw, not a straight line.
        mote.style.top = `${from.y + (to.y - from.y) * along - rise * 4 * along * (1 - along)}px`;
        if (along >= 1) {
            land(false);
            return;
        }
        requestAnimationFrame(step);
    };
    requestAnimationFrame(step);
}
function say() {
    caughtSoFar.textContent = caught === 1 ? "One caught." : `${caught} caught.`;
}
function finish() {
    playing = false;
    dozing.classList.remove("playing");
    motes.replaceChildren();
    greet(ASLEEP);
    wake.hidden = false;
    caughtSoFar.textContent =
        caught === 0
            ? "It yawns and settles back down."
            : `${caught === THROWS ? "Every one" : `${caught} of ${THROWS}`} caught. It yawns and settles back down.`;
}
wake.addEventListener("click", () => {
    playing = true;
    caught = 0;
    thrown = 0;
    wake.hidden = true;
    caughtSoFar.textContent = "";
    greet(AWAKE);
    dozing.classList.add("playing");
    setTimeout(throwOne, 450);
});
// --- Settling in ------------------------------------------------------------
const now = new Date();
document.body.dataset["part"] = partOfClock(now.getHours());
drawGarden(garden, partOfClock(now.getHours()), [], 1);
places.replaceChildren(...choose(now).map(placeCard));
showFact();
seat();
new ResizeObserver(seat).observe(scene);
document.getElementById("more-places")?.addEventListener("click", () => {
    walked += SHOWN;
    places.replaceChildren(...choose(new Date(), walked).map(placeCard));
});
document.getElementById("another-fact")?.addEventListener("click", showFact);
