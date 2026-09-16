# The macOS and Windows shells

Both were built in September 2026 and withdrawn after 0.9.2. This is what they
cost, what they taught, and what is left standing.

The code is not gone, only unbuilt. `66ea5b3` (v0.9.2) is the last commit that
has it:

```sh
git checkout 66ea5b3 -- macos windows scripts/macos-app
```

That restores two crates of about 3,200 lines each, plus the workspace members,
the CI and release jobs, and the `cfg(windows)` bundled-SQLite line in
`core/Cargo.toml`.

## Why they were withdrawn

Not the porting work itself, which mostly went as planned. Two things:

- **Neither could be run here.** The dev machine is Linux. Every fix for either
  shell was written blind, tagged, released, and tried by someone else on their
  own machine — a loop measured in days, on a browser that changes daily. Six
  of the eight commits after the shells landed were fixes for faults that
  would have been obvious in a minute in front of the thing.
- **Three engines is three browsers.** WebKitGTK, WKWebView and WebView2 agree
  on very little past `Navigate`. Tabs, downloads, find, zoom, failure pages
  and session restore each landed three times, and each of the three has its
  own gaps (below). Every new browser feature from here would have cost the
  same.

Both shells were working browsers when they stopped: tabs, bookmarks, find in
page, zoom, downloads, history, a restored session, the keyboard, failure
pages, and the wisp in its nook. They were not abandoned as unfinished.

## What was still wrong when it stopped

Reported on 0.9.2 and never diagnosed:

- **macOS: hovering the wisp shows no caption.** The nook's `NSTrackingArea`
  calls `hovering_wisp`, which sends `ToChrome::Caption` to the toolbar. Two
  untested suspects: the tracking area is `ActiveInKeyWindow` on a view sitting
  over a `WKWebView` that installs tracking areas of its own; or the message
  reaches nothing, which is the same fault as `902b77e` — the `glimmerwood://`
  handler answering only one of the chrome's two views.
- **Windows: won't start, reporting that something can't be found.** Check
  `WebView2Loader.dll` beside the exe first. `windows/build.rs` copies it out
  of the bindings' `OUT_DIR` on a best-effort basis and only prints a cargo
  warning when it can't find it, so a build can succeed and produce something
  that cannot start. A missing WebView2 *runtime* is a different failure, with
  its own message box naming it.

Known gaps, by design or by API:

| | macOS | Windows |
| --- | --- | --- |
| Sound and mute | No. `isPlayingAudio` is private on WKWebView, so the dose engine misses "sound keeps you present" and the column offers no mute | Yes, via `ICoreWebView2_8` |
| Favicons | No. WebKit offers no way to ask a site for its icon; every tab keeps its letter | Yes, via `ICoreWebView2_15` |
| Find in page | WebKit's own, but it reports *whether* it found something, never how many | Injected script: one obscured property, `CSS.highlights` with a constructed sheet, spans where that API is missing. Does not reach subframes or shadow roots |
| Title and load state | Polled every two seconds; KVO observers were never written | Evented |

## What held up

The three bets in the original plan were all right, and all of them survive the
withdrawal:

1. **The chrome is a web page.** `ui/` went to both shells unchanged. Only the
   message channel differs — `WKScriptMessageHandler` and `WebMessageReceived`
   against WebKitGTK's named handler — and that was four lines in `shared.ts`.
2. **The wisp draws against ~18 operations.** `core/src/canvas.rs` came out of
   this, and all 1,027 lines of the drawing with it. It now has three surfaces
   behind it — cairo for the window, SVG for the welcome page, a software
   rasteriser for the tests — and the mood tests in `raster/` hold the face to
   all of them. This was worth doing on its own.
3. **The core is closed.** Eighteen modules that depend only on each other, no
   toolkit anywhere in their dependencies. The two GLib calls that were hiding
   in there (`attention::now` for the UTC offset, `settings::path` for the
   config directory) are injected now, which is what this repo's own rule about
   the clock always asked for.

What was never done is step 3 of the plan: a `WebView` trait with the engines
behind it. The shells were written without it, sharing `core/src/tabs.rs`,
`core/src/failure.rs` and `core/src/zoom.rs` instead. A third copy of the
navigation plumbing is exactly the cost that trait would have saved, and
anyone picking this up should write it first rather than last.

## What each engine gives you

Everything the GTK window asks of WebKitGTK, and its counterpart:

| Glimmerwood needs | WKWebView | WebView2 |
| --- | --- | --- |
| `load_uri`, `reload`, `stop_loading` | `load(_:)`, `reload()`, `stopLoading()` | `Navigate`, `Reload`, `Stop` |
| `go_back`, `go_forward`, `can_go_*` | `goBack()`, `canGoBack` | `GoBack`, `CanGoBack` |
| `uri`, `title`, `is_loading`, `estimated_load_progress` | `url`, `title`, `isLoading`, `estimatedProgress` | `Source`, `DocumentTitle`, `NavigationCompleted` |
| `evaluate_javascript` | `evaluateJavaScript` | `ExecuteScriptAsync` |
| `is_playing_audio`, `is_muted` | `isPlayingAudio` (private), `isMuted` | `IsDocumentPlayingAudio`, `IsMuted` |
| `connect_decide_policy` | `WKNavigationDelegate.decidePolicyFor` | `NavigationStarting` |
| `connect_load_changed`, `load_failed`, TLS failures | `didCommit`, `didFail`, `didReceive challenge` | `NavigationStarting`/`Completed`, `ServerCertificateErrorDetected` |
| `connect_web_process_terminated` | `webViewWebContentProcessDidTerminate` | `ProcessFailed` |
| script message handler | `WKScriptMessageHandler` | `WebMessageReceived` |
| `glimmerwood://` scheme | `WKURLSchemeHandler` | `AddWebResourceRequestedFilter` |
| find in page | `find(_:configuration:)` | none — inject it |

## Things that cost a release each to find

Written down because none of them is obvious from the API, and all of them cost
a round trip to someone else's machine:

- **A Windows GUI program must not be built as a console program.** Otherwise a
  console window stands open beside the browser for as long as it runs, and on
  a failed start both close before anything can be read. With the console gone,
  nothing printed goes anywhere: a start that fails and a panic both need a
  message box, neither of which has a window to hang off yet.
- **WebView2 told nothing about a profile folder fills one beside the binary** —
  caches, cookies and its own crash reports, in whatever folder the zip was
  unpacked into. Told `%LOCALAPPDATA%` and refused, it used to refuse to open at
  all. It needs asking twice: once with the folder, then with nowhere named.
  A known-folder lookup that returns a relative path counts as no answer, since
  the engine reads a relative path as relative to the program.
- **The nook is placed by the toolbar, not by the shell.** Handed a point as
  well as a size, a Windows layered window moves to it in *screen* coordinates,
  which for a default point is the corner of the desktop. The wisp ended up
  there.
- **A custom scheme handler must answer every view that may ask.** Both chrome
  views may ask for anything carried in the binary; a tab is still the web and
  may ask only for Home and Settings. Answering only the toolbar drops the
  column's request with no failure attached, and WebKit sits on a navigation
  that never finishes — an empty white strip, for four releases.
- **Neither shell reported input at first**, so the companion's idea of presence
  never left "away" and the wisp slept through everything. macOS passes on every
  event it sees without saying what it was; Windows reports a key on arrival and
  otherwise asks the system every two seconds when it last saw any input,
  counting it only while its own window is in front.
- **The session is worth writing when a title arrives**, not only when a page
  commits — a page has no title yet at commit, so restored tabs came back as
  bare addresses.
- **A restored tab should have no engine behind it.** A window of eight sleeping
  tabs then costs what a window of one costs, and a controller is made the
  moment a tab is first asked for. A sleeping tab names no address, so the wisp
  counts none of them.
- **An app bundle with an empty `Resources` folder looks fine until it doesn't.**
  Both workflows were made to fail on a missing `.icns`; a missing icon is easy
  to lose and hard to notice.

## What it costs beyond the code

- An Apple Developer Program membership, $99/yr, for signing and notarisation.
  Unsigned, Gatekeeper refuses a double-click. macOS CI is free on GitHub
  Actions for a public repository.
- A Windows signing certificate, or every download warns through SmartScreen.
- GPL-3.0 is fine for direct downloads on both. It is the App Store it cannot go
  through, which only matters if iOS ever follows.
- A machine of each to run it on. This is the one that actually ended it.
