// A site asking to use the camera, the microphone, the screen or the
// person's location: a line in the bar, never over the page, and it never
// takes the caret. Allow is remembered for the site; Not now is not.

import type { ToChrome, Want } from "../protocol.gen.js";
import { element, send } from "./shared.js";

type Permission = Extract<ToChrome, { type: "permission" }>;

const line = document.createElement("div");
line.className = "permission";
line.setAttribute("role", "group");
line.setAttribute("aria-live", "polite");
line.hidden = true;

const words = document.createElement("span");
words.className = "permission-words";

const notNow = document.createElement("button");
notNow.type = "button";
notNow.className = "permission-not-now";
notNow.textContent = "Not now";

const allow = document.createElement("button");
allow.type = "button";
allow.className = "permission-allow";
allow.textContent = "Allow";

line.append(words, notNow, allow);
element("bookmark", HTMLButtonElement).before(line);

let asking = 0;

function answer(allowed: boolean): void {
  if (!asking) return;
  send({ type: "answer_permission", id: asking, allow: allowed });
  asking = 0;
  line.hidden = true;
}
notNow.addEventListener("click", () => answer(false));
allow.addEventListener("click", () => answer(true));

const NOUNS: Record<Exclude<Want, "location">, string> = {
  camera: "camera",
  microphone: "microphone",
  screen: "screen",
};

// "your camera and microphone", or "where you are".
function wanting(wants: Want[]): string {
  if (wants.includes("location")) return "to know where you are";
  const nouns = wants.flatMap((want) => (want === "location" ? [] : [NOUNS[want]]));
  const listed = nouns.length > 1 ? `${nouns.slice(0, -1).join(", ")} and ${nouns[nouns.length - 1]}` : nouns[0];
  return `to use your ${listed}`;
}

export function showPermission(message: Permission): void {
  if (message.site === null) {
    if (message.id === asking) {
      asking = 0;
      line.hidden = true;
    }
    return;
  }
  asking = message.id;
  const site = document.createElement("strong");
  site.textContent = message.site;
  words.replaceChildren(site, ` would like ${wanting(message.wants)}.`);
  allow.title = `${message.site} won't need to ask again`;
  line.hidden = false;
}
