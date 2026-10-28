A way to say how it's going, and files open as themselves.

- **How did it feel?** Settings has a new section with a link to a short form
  on GitHub, to say how your days with Glimmerwood went, good or bad. It opens
  in a new tab, and you'll need a GitHub account.
- **Files open as themselves.** An HTML file opened from a file manager, or
  named on the command line, now loads directly. Before, it was taken as
  something typed into the address field and searched for.
- **Signed bundles.** The release carries a signature beside the bundle, so
  Slipstream's installer can check who made it.

## Getting it

A Flatpak bundle, x86_64, needing [Flathub](https://flathub.org/setup) for the
GNOME runtime:

```sh
curl -LO https://github.com/peterwalker78/glimmerwood/releases/latest/download/glimmerwood-x86_64.flatpak
flatpak install --user glimmerwood-x86_64.flatpak
```

Checksums are in `SHA256SUMS`.
