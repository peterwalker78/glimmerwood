// One good place, as a card. Home and the welcome page offer the same thing
// in the same shape, so they show it with the same code.

export type PlaceCard = { name: string; line: string; url: string; yours?: boolean };

export function letter(text: string): HTMLSpanElement {
  const tile = document.createElement("span");
  tile.className = "tile";
  tile.setAttribute("aria-hidden", "true");
  tile.textContent = (text.replace(/^www\./, "").trim().charAt(0) || "·").toUpperCase();
  return tile;
}

export function span(className: string, text: string): HTMLSpanElement {
  const s = document.createElement("span");
  s.className = className;
  s.textContent = text;
  return s;
}

export function placeCard(place: PlaceCard): HTMLLIElement {
  const item = document.createElement("li");
  item.className = "place";
  const link = document.createElement("a");
  link.href = place.url;
  link.append(letter(place.name), span("name", place.name), span("line", place.line));
  if (place.yours) link.append(span("yours", "You come back here"));
  item.append(link);
  return item;
}
