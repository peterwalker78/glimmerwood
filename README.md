# Wisp

A quiet web browser for Linux with a living companion.

A small light called the wisp lives in a nook at the end of the toolbar. It
notices which parts of the web restore you and which wear you down, and shows
it the way weather shows: bright and breathing gently after time away or
somewhere good, clouded and sleepy after a long stretch of endless feeds. It
never scolds, never blocks a page and never pops up a dialog about your
habits. Hover it to see what is moving it; click it to see its days.

Wisp is in early development.

## What it does

- **The wisp.** Its look follows a *dose* that rises with time on draining
  sites and falls with rest. Being away from the screen restores it fastest;
  restorative sites help too. Late at night it winds down, and on private
  sites it turns away and gives you privacy.
- **Home.** Every new tab opens a local page with a garden that grows on good
  days and never wilts, a greeting that notices what went well, good places to
  go, your bookmarks, and the wisp's days: today's shape, the week and the
  weeks before.
- **Reputation lists.** About 2,300 sites sorted into lists from strongly
  draining to strongly nourishing, following published research on wellbeing
  online (the sources are cited in `core/data/reputation.toml`). A site on no
  list holds steady. You can move any site in your own `reputation.toml`
  (`~/.config/wisp/`, or
  `~/.var/app/io.github.peterwalker78.Wisp/config/wisp/` in the Flatpak),
  which overrides the bundled lists.
- **A plain browser underneath.** WebKitGTK, tabs in a resizable column,
  HTTPS first, find in page, tracking prevention.

## Privacy

Everything Wisp knows stays on your computer. It makes no network requests of
its own: no telemetry, no update checks, no remote classification. The dose
history records the reputation list entry a page matched, never the address
you visited, and sites on the private list are never named anywhere.

## Building

Wisp is written in Rust on GTK 4 and WebKitGTK 6.0, with a small TypeScript
interface compiled by the native TypeScript 7 compiler (fetched and checked by
the build scripts; no Node needed).

Requirements: Rust (stable), GTK 4.20 or later, WebKitGTK 6.0,
`glib-compile-resources`, SQLite, and `curl` for fetching the compiler.

```sh
scripts/dev      # build and run from the source tree
scripts/check    # formatting, clippy, tests and the TypeScript build
scripts/flatpak  # build and install the Flatpak for your user
```

If a distrobox named `wisp` exists, `scripts/dev` and `scripts/check` run
inside it. `scripts/flatpak` needs Flathub's `org.flatpak.Builder`.

`wisp --feel-lab` plays a scripted day at speed, to watch how the wisp moves
without living through a day of browsing (`--from=HH:MM`, `--speed=N`).

AI coding tools are used in writing Wisp's code.

## Licence

GPL-3.0-or-later. See `COPYING`.
