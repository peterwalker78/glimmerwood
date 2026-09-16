**Glimmerwood is a Linux browser.** The macOS and Windows builds are withdrawn
from 0.10.0. Both were working browsers — tabs, bookmarks, find, zoom,
downloads, a restored session and the wisp in its nook — but neither could be
developed against on a Linux machine, and three engines that agree on little
past `Navigate` meant every browser feature was written three times.
[`docs/ports/`](https://github.com/peterwalker78/glimmerwood/tree/main/docs/ports)
is the record: what each engine gives you, the faults that cost a release each
to find, and what picking either up again would take.

Nothing else changes. The Linux build is the same browser, the same wisp and
the same lists.

## Getting it

A Flatpak bundle, x86_64, needing [Flathub](https://flathub.org/setup) for the
GNOME runtime:

```sh
curl -LO https://github.com/peterwalker78/glimmerwood/releases/latest/download/glimmerwood-x86_64.flatpak
flatpak install --user glimmerwood-x86_64.flatpak
```

Checksums are in `SHA256SUMS`.
