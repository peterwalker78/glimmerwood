# Installing Glimmerwood

[← back to the README](../README.md)

Glimmerwood is free and open source. Install it, make it your default browser, and
then just use the web as you normally would. There's nothing to set up and
nothing to learn: the wisp starts noticing straight away, and your garden can
grow its first plant tomorrow.

**Install the Flatpak** (x86_64 Linux, needs [Flathub](https://flathub.org/setup)
for the GNOME runtime):

```sh
curl -LO https://github.com/peterwalker78/glimmerwood/releases/latest/download/glimmerwood-x86_64.flatpak
flatpak install --user glimmerwood-x86_64.flatpak
```

Then open **Glimmerwood** from your applications. To make it your default browser,
choose it under *Default Applications* in your desktop's settings.


> **Glimmerwood was called Wisp** in its first release, until we found another
> browser already had the name. Version 0.2.0 has a new app ID, so it installs
> beside Wisp rather than replacing it: remove the old one with
> `flatpak uninstall --user io.github.peterwalker78.Wisp`.


> Glimmerwood is young. It's already a comfortable everyday browser for most of the
> web, and it's growing quickly. If something doesn't work, please
> [open an issue](https://github.com/peterwalker78/glimmerwood/issues).

## Windows and macOS

Glimmerwood is a Linux browser. Shells for both were built and then withdrawn
after 0.9.2; what was learned doing it, and what it would take to pick either
up again, is in [`docs/ports/`](../docs/ports).


<br>

## If something doesn't work

Please [open an issue](https://github.com/peterwalker78/glimmerwood/issues).
Glimmerwood is young: it's already a comfortable everyday browser for most of
the web, and it's growing quickly.
