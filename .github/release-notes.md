## Getting it

**Linux** — a Flatpak bundle, x86_64, needing [Flathub](https://flathub.org/setup)
for the GNOME runtime:

```sh
curl -LO https://github.com/peterwalker78/glimmerwood/releases/latest/download/glimmerwood-x86_64.flatpak
flatpak install --user glimmerwood-x86_64.flatpak
```

**Windows** — unzip `glimmerwood-windows-x64.zip` and run `Glimmerwood.exe`,
keeping it beside `WebView2Loader.dll`. It needs the WebView2 runtime, which
Windows 11 and current Windows 10 already have; if a machine hasn't got it, the
app says so. Unsigned, so SmartScreen warns once: *More info*, then *Run anyway*.

**macOS** — unzip `glimmerwood-macos.zip` and drag **Glimmerwood** to
Applications. macOS 11 or later, Apple silicon or Intel. Unsigned, so Gatekeeper
refuses a double-click. Right-click the app and choose **Open**, or clear the
flag first:

```sh
xattr -dr com.apple.quarantine /Applications/Glimmerwood.app
```

Find in page is still to come on the Windows and macOS builds, and macOS
can't yet tell when a tab is making a sound. Checksums for every file are in
`SHA256SUMS`.
