# Building Glimmerwood

[← back to the README](../README.md)

Glimmerwood is written in Rust, on the engine the system already has: GTK 4 and
WebKitGTK 6.0. Nothing is bundled. The wisp itself is drawn natively rather
than in a web page, so the whole browser idles at around one percent of a CPU
core with the wisp breathing. The toolbar and Home are small TypeScript pages
compiled by the native TypeScript 7 compiler, so there's no Node in the build.

The workspace splits the browser from the machine it runs on. `core/` is the
dose model, the lists, the garden, the diary, the tabs, the protocol and the
whole of the wisp's drawing, with no window toolkit anywhere in its
dependencies; `app/` is the GTK shell on top of it, holding only what the
window toolkit does. `raster/` draws the wisp into pixels with no toolkit under
it, which is what the tests hold its face to.

Requirements: Rust (stable), GTK 4.20 or later, WebKitGTK 6.0,
`glib-compile-resources`, SQLite, and `curl` to fetch the compiler.

```sh
scripts/dev      # build and run from the source tree
scripts/check    # formatting, clippy, tests and the TypeScript build
scripts/flatpak  # build and install the Flatpak for your user
scripts/bundle   # build a single-file Flatpak bundle for release
scripts/site     # build the welcome page; --publish puts it online
```

If a distrobox named `glimmerwood` exists, `scripts/dev` and `scripts/check` run
inside it. `scripts/flatpak` and `scripts/bundle` need Flathub's
`org.flatpak.Builder`.

`glimmerwood --feel-lab` plays a scripted day at high speed, so you can watch the
wisp's moods without living through a day of browsing (`--from=HH:MM`,
`--speed=N`).

AI coding tools are used in writing Glimmerwood's code.
