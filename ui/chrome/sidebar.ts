// The column of tabs down the left. Its width is the user's: they drag its
// edge (a native handle), and titles appear once there's room for them.

import type { TabInfo } from "../protocol.gen.js";
import { dragFrom, element, receive, send } from "./shared.js";
import { TabList } from "./tabs.js";

const column = element("column", HTMLDivElement);
const strip = element("tabs", HTMLDivElement);
const list = new TabList(strip, send);

// Icons arrive on their own, only when they change.
const icons = new Map<number, string>();
let tabs: TabInfo[] = [];
let selected = 0;

function render(): void {
  for (const id of icons.keys()) {
    if (!tabs.some((tab) => tab.id === id)) icons.delete(id);
  }
  const hadFocus = strip.contains(document.activeElement);
  list.render(tabs, selected, icons);
  if (hadFocus) list.focusSelected();
}

element("new-tab", HTMLButtonElement).addEventListener("click", () => send({ type: "new_tab" }));
dragFrom([column, strip]);

receive((message) => {
  switch (message.type) {
    case "tabs":
      tabs = message.tabs;
      selected = message.selected;
      render();
      break;
    case "tab_icon":
      if (message.icon) icons.set(message.id, message.icon);
      else icons.delete(message.id);
      render();
      break;
    case "state":
    case "focus_address":
    case "find":
    case "found":
    case "window":
    case "caption":
    case "ask":
    case "wisp":
      // The toolbar's business.
      break;
    default: {
      const unhandled: never = message;
      throw new Error(`unhandled message ${JSON.stringify(unhandled)}`);
    }
  }
});

send({ type: "ready", view: "sidebar" });
