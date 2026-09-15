// Ratings in words, and the five-stop slider from draining to restoring.
// Shared by the wisp's question in the toolbar and the Settings page, so the
// two always say the same thing.

import type { Rating } from "../protocol.gen.js";

// Left to right along the slider.
export const SCALE: readonly Rating[] = [
  "drains_a_lot",
  "drains_a_little",
  "neither",
  "restores_a_little",
  "restores_a_lot",
];

export const WORDS: Record<Rating, string> = {
  drains_a_lot: "Drains me a lot",
  drains_a_little: "Drains me a little",
  neither: "Neither",
  restores_a_little: "Restores me a little",
  restores_a_lot: "Restores me a lot",
  news: "News",
  private: "Private",
  unrated: "Unrated",
};

// A slider's position for a rating; the middle for one off the scale.
export function position(rating: Rating): number {
  const at = SCALE.indexOf(rating);
  return at < 0 ? 2 : at;
}

export function atPosition(value: string): Rating {
  return SCALE[Math.round(Number(value))] ?? "neither";
}

// A slider from "Drains me" to "Restores me", with the chosen words between
// the ends. `onChoose` hears each rating the user settles on.
export function slider(label: string, rating: Rating, onChoose: (rating: Rating) => void): HTMLDivElement {
  const box = document.createElement("div");
  box.className = "scale";
  const input = document.createElement("input");
  input.type = "range";
  input.min = "0";
  input.max = String(SCALE.length - 1);
  input.step = "1";
  input.setAttribute("aria-label", label);
  const ends = document.createElement("div");
  ends.className = "scale-ends";
  const left = document.createElement("span");
  left.textContent = "Drains me";
  const chosen = document.createElement("span");
  chosen.className = "scale-choice";
  chosen.setAttribute("aria-hidden", "true");
  const right = document.createElement("span");
  right.textContent = "Restores me";
  ends.append(left, chosen, right);
  box.append(input, ends);
  setSlider(box, rating);
  input.addEventListener("input", () => {
    box.classList.remove("off");
    const now = atPosition(input.value);
    chosen.textContent = WORDS[now];
    input.setAttribute("aria-valuetext", WORDS[now]);
  });
  input.addEventListener("change", () => onChoose(atPosition(input.value)));
  return box;
}

// Show `rating` on a slider made by `slider`. A rating off the scale leaves
// the thumb resting in the middle, faded, with its name shown.
export function setSlider(box: HTMLElement, rating: Rating): void {
  const input = box.querySelector("input");
  const chosen = box.querySelector(".scale-choice");
  if (!input || !chosen) return;
  input.value = String(position(rating));
  chosen.textContent = WORDS[rating];
  input.setAttribute("aria-valuetext", WORDS[rating]);
  box.classList.toggle("off", !SCALE.includes(rating));
}
