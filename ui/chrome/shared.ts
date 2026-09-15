// What both chrome pages share: finding elements, talking to the core, and
// dragging the window from empty chrome.

import type { ToChrome, ToCore } from "../protocol.gen.js";

declare global {
  interface Window {
    webkit: { messageHandlers: { wisp: { postMessage(message: ToCore): void } } };
    // Not `wisp`: the wisp button's id would shadow it until the page loads.
    wispChrome: { receive(message: ToChrome): void };
  }
}

export function element<T extends HTMLElement>(id: string, kind: new () => T): T {
  const found = document.getElementById(id);
  if (!(found instanceof kind)) throw new Error(`chrome page has no #${id}`);
  return found;
}

export function send(message: ToCore): void {
  window.webkit.messageHandlers.wisp.postMessage(message);
}

export function receive(handler: (message: ToChrome) => void): void {
  window.wispChrome = { receive: handler };
}

// Empty chrome drags the window, and a double-click there maximises it.
// "Empty" means the press landed on an element marked as a drag area, not on
// anything inside it that does something. The move only starts once the
// pointer has travelled a little with the button held, so a click that just
// misses a tab is still a click, and a double-click still reaches us.
const DRAG_THRESHOLD = 4;

export function dragFrom(areas: HTMLElement[]): void {
  let pressed: { x: number; y: number } | null = null;
  for (const area of areas) {
    area.addEventListener("mousedown", (event) => {
      pressed = event.button === 0 && event.target === area && event.detail === 1
        ? { x: event.screenX, y: event.screenY }
        : null;
    });
    area.addEventListener("dblclick", (event) => {
      if (event.target === area) send({ type: "toggle_maximize" });
    });
  }
  window.addEventListener("mousemove", (event) => {
    if (!pressed) return;
    if ((event.buttons & 1) === 0) {
      pressed = null;
      return;
    }
    if (Math.hypot(event.screenX - pressed.x, event.screenY - pressed.y) >= DRAG_THRESHOLD) {
      pressed = null;
      send({ type: "begin_move" });
    }
  });
  window.addEventListener("mouseup", () => {
    pressed = null;
  });
}
