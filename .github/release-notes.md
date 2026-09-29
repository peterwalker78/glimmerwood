The wisp hums along, and good news for your feed reader.

- **The wisp hums along.** While the page in front of you plays sound, a
  small note drifts up from beside the wisp every few seconds, in the same ink
  as its z's. It stops when the sound does.
- **Good news in Newsboat.** If you read feeds in
  [Newsboat](https://newsboat.org), Settings can add twelve good news feeds:
  seven that report what's working, two light ones and three for awe. They go
  at the end of your `urls` file, tagged `glimmerwood`, and one click takes out
  exactly those lines again. Glimmerwood fetches nothing itself.
- **This week's pages fold a day at a time.** Today is open, every other day
  waits behind its name and page count, and a heavy day shows its first twelve
  pages with the rest a click away.
- **The wisp's breath keeps its rhythm** when it slows at night or as the wisp
  dozes, where before it jumped to a different point in the breath.
- **A smaller program**: 2.3 MB, down from 3.8 MB.
- **Slipstream installs Glimmerwood again.** Releases carry
  `glimmerwood-x86_64.flatpak.sha256` beside `SHA256SUMS`, which is the file
  Slipstream's installer checks the download against.

## Getting it

A Flatpak bundle, x86_64, needing [Flathub](https://flathub.org/setup) for the
GNOME runtime:

```sh
curl -LO https://github.com/peterwalker78/glimmerwood/releases/latest/download/glimmerwood-x86_64.flatpak
flatpak install --user glimmerwood-x86_64.flatpak
```

Checksums are in `SHA256SUMS`.
