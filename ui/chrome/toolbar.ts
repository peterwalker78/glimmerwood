// The toolbar across the top of the window: back, forward, reload, the
// address field, the wisp's nook, and window buttons while the window floats.
// The wisp's caption opens below the nook, floating over the page.

import type { ToChrome } from "../protocol.gen.js";
import { renderCaption } from "./caption.js";
import { dragFrom, element, receive, send } from "./shared.js";

type State = Extract<ToChrome, { type: "state" }>;

const bar = element("bar", HTMLElement);
const back = element("back", HTMLButtonElement);
const forward = element("forward", HTMLButtonElement);
const reload = element("reload", HTMLButtonElement);
const form = element("address-form", HTMLFormElement);
const address = element("address", HTMLInputElement);
const security = element("security", HTMLSpanElement);
const findForm = element("find-form", HTMLFormElement);
const find = element("find", HTMLInputElement);
const findSummary = element("find-summary", HTMLSpanElement);
const star = element("bookmark", HTMLButtonElement);
const progress = element("progress", HTMLDivElement);
const windowButtons = element("window-buttons", HTMLDivElement);
const nook = element("wisp", HTMLButtonElement);
const caption = element("caption", HTMLElement);
const captionLines = element("caption-lines", HTMLUListElement);

let current: State | null = null;
// Once the user has typed in the address field, the page loading underneath
// must not overwrite it. Merely focusing the field doesn't count.
let edited = false;

// The core sizes the toolbar webview to what this reports: the bar pages sit
// below and, while the caption is open, room for it over the page. It draws
// the wisp natively over the nook's rectangle, measured from the top right so
// it stays put as the window resizes.
let reported = "";
function reportLayout(): void {
  const height = Math.ceil(bar.getBoundingClientRect().height);
  const home = nook.getBoundingClientRect();
  if (!caption.hidden) {
    caption.style.right = `${Math.max(8, Math.round(innerWidth - home.right))}px`;
  }
  const layout = {
    type: "toolbar_layout" as const,
    height,
    overlay_height: caption.hidden ? height : Math.ceil(caption.getBoundingClientRect().bottom) + 12,
    nook_right: Math.max(0, Math.round(innerWidth - home.right)),
    nook_top: Math.round(home.top),
    nook_width: Math.round(home.width),
    nook_height: Math.round(home.height),
  };
  const key = JSON.stringify(layout);
  if (key === reported) return;
  reported = key;
  send(layout);
}
new ResizeObserver(reportLayout).observe(bar);
new ResizeObserver(reportLayout).observe(nook);
new ResizeObserver(reportLayout).observe(caption);
// The window buttons come and go beside the nook, moving it.
new MutationObserver(reportLayout).observe(windowButtons, { attributes: true });

// The caption: shown the moment the wisp is hovered or focused. The dose moves
// every second or so; touching the page each time would make WebKit render a
// frame for nothing, so only an open caption is kept up to date.
let latest: Extract<ToChrome, { type: "wisp" }> | null = null;
let shownCaption = "";
function updateCaption(): void {
  if (!latest) return;
  // Everything but the dose itself, which moves every second.
  const key = JSON.stringify({ ...latest, dose: 0 });
  if (key === shownCaption) return;
  shownCaption = key;
  renderCaption(captionLines, latest);
}

let hovering = false;
let focused = false;
function showCaption(): void {
  const open = hovering || focused;
  if (caption.hidden !== !open) {
    if (open) updateCaption();
    caption.hidden = !open;
    reportLayout();
  }
}
// Only keyboard focus opens it: a click on the nook focuses it too, and the
// open caption would then cover the top of the page until focus moved.
nook.addEventListener("focus", () => {
  focused = nook.matches(":focus-visible");
  showCaption();
});
// The native wisp takes pointer clicks; this is Enter or Space on the nook.
nook.addEventListener("click", () => send({ type: "show_wisp" }));
nook.addEventListener("blur", () => {
  focused = false;
  showCaption();
});

function render(state: State): void {
  back.disabled = !state.can_go_back;
  forward.disabled = !state.can_go_forward;

  reload.classList.toggle("loading", state.loading);
  reload.setAttribute("aria-label", state.loading ? "Stop" : "Reload");
  reload.title = state.loading ? "Stop" : "Reload (Ctrl+R)";

  if (!edited) address.value = state.uri;
  security.hidden = state.security !== "not_secure";
  star.hidden = !state.can_bookmark;
  star.classList.toggle("on", state.bookmarked);
  star.setAttribute("aria-pressed", String(state.bookmarked));
  star.title = state.bookmarked ? "Remove bookmark (Ctrl+D)" : "Bookmark (Ctrl+D)";

  progress.hidden = !state.loading;
  progress.style.width = state.loading ? `${Math.max(0.04, state.progress) * 100}%` : "0";

  document.title = state.title || "Glimmerwood";
}

back.addEventListener("click", () => send({ type: "back" }));
forward.addEventListener("click", () => send({ type: "forward" }));
reload.addEventListener("click", () => send({ type: current?.loading ? "stop" : "reload" }));
star.addEventListener("click", () => send({ type: "toggle_bookmark" }));
element("home", HTMLButtonElement).addEventListener("click", () => send({ type: "go_home" }));
element("minimize", HTMLButtonElement).addEventListener("click", () => send({ type: "minimize" }));
element("maximize", HTMLButtonElement).addEventListener("click", () =>
  send({ type: "toggle_maximize" }),
);
element("close", HTMLButtonElement).addEventListener("click", () => send({ type: "close_window" }));
dragFrom([bar]);

address.addEventListener("focus", () => address.select());
address.addEventListener("input", () => {
  edited = true;
});
address.addEventListener("blur", () => {
  edited = false;
  if (current) render(current);
});
address.addEventListener("keydown", (event) => {
  if (event.key !== "Escape") return;
  edited = false;
  address.value = current?.uri ?? "";
  address.blur();
  send({ type: "focus_page" });
});

form.addEventListener("submit", (event) => {
  event.preventDefault();
  const input = address.value.trim();
  if (!input) return;
  edited = false;
  address.blur();
  send({ type: "navigate", input });
});

// Find in page. Every keystroke searches from the top; Enter moves on to the
// next match and Shift+Enter back. The core answers with the count in words.
function openFind(open: boolean): void {
  findForm.hidden = !open;
  if (!open) {
    findSummary.textContent = "";
    return;
  }
  find.focus();
  find.select();
  // Closing cleared the highlights; bring them back for what's still typed.
  if (find.value) send({ type: "find", query: find.value });
}
find.addEventListener("input", () => {
  if (!find.value) findSummary.textContent = "";
  send({ type: "find", query: find.value });
});
find.addEventListener("keydown", (event) => {
  if (event.key === "Escape") {
    event.preventDefault();
    send({ type: "close_find" });
  } else if (event.key === "Enter") {
    event.preventDefault();
    send({ type: "find_next", backwards: event.shiftKey });
  }
});
element("find-previous", HTMLButtonElement).addEventListener("click", () =>
  send({ type: "find_next", backwards: true }),
);
element("find-next", HTMLButtonElement).addEventListener("click", () =>
  send({ type: "find_next", backwards: false }),
);
element("find-close", HTMLButtonElement).addEventListener("click", () => send({ type: "close_find" }));

receive((message) => {
  switch (message.type) {
    case "state":
      current = message;
      render(message);
      break;
    case "focus_address":
      address.focus();
      address.select();
      break;
    case "find":
      openFind(message.open);
      break;
    case "found":
      if (message.query === find.value) findSummary.textContent = message.summary;
      break;
    case "window":
      windowButtons.hidden = !message.floating;
      break;
    case "caption":
      hovering = message.open;
      showCaption();
      break;
    case "wisp":
      latest = message;
      if (!caption.hidden) updateCaption();
      break;
    case "tabs":
    case "tab_icon":
      // The tab column's business.
      break;
    default: {
      const unhandled: never = message;
      throw new Error(`unhandled message ${JSON.stringify(unhandled)}`);
    }
  }
});

send({ type: "ready", view: "toolbar" });
reportLayout();
