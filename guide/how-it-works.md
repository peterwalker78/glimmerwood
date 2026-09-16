# How Glimmerwood works

The wisp grows heavier while you're somewhere that drains you and lighter
while you rest. This is exactly how.

[← back to the README](../README.md)

## The dose

Behind the wisp is a single number between 0 and 1, called the **dose**. Think
of it as how heavy the wisp feels. It rises while you're on draining sites and
falls while you rest, and the wisp's look follows it: bright below 0.2, busy
up to 0.5, clouded up to 0.75, and sleepy beyond.

- **On a draining site** it climbs steadily. A long unbroken session reaches
  sleepy in about 50 minutes.
- **Short visits cost less than long sessions.** The first ten minutes count
  half, and after half an hour each minute counts a little more. A five-minute
  break starts you fresh.
- **Away from the screen** it halves every 25 minutes. This is the fastest
  way to recover, on purpose.
- **On a restorative site** it recovers too, just more slowly than being away.
  Everyday sites (your bank, your email, a map) recover it more slowly still.
- **Late at night** (11pm to 5am, or your own hours in Settings) draining time
  counts one and a half times, and everyday sites stop counting as rest.
- **A new day** starts fresh at 5am, carrying over at most a little of a very
  heavy yesterday.

Glimmerwood only counts time you're actually there: if you haven't touched the
keyboard or mouse for four minutes, it treats you as away (unless you're
watching a video).

## The lists

How a site affects the dose depends on which list it's on. Glimmerwood comes
with about 8,000 sites already sorted from published research into wellbeing
online: the 10,000 most-visited sites on the web, checked one by one, plus
long lists of gambling and adult sites. The studies behind each group are
cited in [`core/data/reputation.toml`](../core/data/reputation.toml).


| List | Weight | Examples |
|---|---|---|
| Draining, strong | −1 | endless short-video feeds, gambling |
| Draining, mild | −0.4 | dating and trading apps, clickbait, gossip |
| News | −0.4 | news sites; the first 15 minutes each day count half |
| Everyday | 0 | banking, government, health services, maps, shops |
| Restoring, mild | +0.5 | reference, learning, making, the arts, puzzles |
| Restoring, strong | +1 | meditation, books and reading, live nature cams |


A site on no list simply holds the wisp steady, and where the research was
mixed, a site went on the milder list.


Everyone is different, so the lists are yours to change. Spend a little while
on a site the wisp hasn't met and it asks, once, how the place leaves you,
with a slider from *drains me* to *restores me*. Only you see the answer; it's
there so the wisp can look after you honestly. Every rating can be changed
later in **Settings** (Ctrl+comma, or the link at the foot of Home), where you
can also look up and rate any site. Ratings are kept in your own
`reputation.toml` (the same list names as the bundled file, such as
`draining-mild` or `nourishing-strong`) in `~/.config/glimmerwood/` (or
`~/.var/app/io.github.peterwalker78.Glimmerwood/config/glimmerwood/` for the
Flatpak), which you can edit by hand too. Changes take effect immediately.

## Home

Every new tab opens **Home**, a calm page made on your computer rather than
fetched from anywhere.

- **A garden that grows on good days.** A quiet day brings moss, a day of
  learning brings ferns, a gentle day brings flowers, and a calm night leaves
  fireflies. A heavy day simply grows nothing. **Nothing ever wilts, and
  missing a day costs you nothing:** there are no streaks to break.
- **A sky that follows the real sun.** Morning, day, evening and night arrive
  when they actually do where you are, rather than when a clock says so.
- **A greeting that notices what went well.** "You took a proper break
  earlier." "Last night stayed calm after dark." Only ever true, and never
  about what went badly.
- **Good places.** Instead of a grid of the sites you visit most — the ones you
  least need reminding of — Home offers somewhere worth going: the places you
  already return to, and fresh suggestions every couple of hours from over 300
  researched corners of the web that leave people better off, from live nature
  cams and free books to museums, music, making and gentle puzzles. They fit
  the time of day and the season, never repeat what you saw earlier today, and
  lean toward calm when the wisp is tired.
- **Your bookmarks,** kept on your computer.

The first time you open Home, the wisp introduces itself:

<br>

## The everyday things

Downloads land in your Downloads folder with a quiet mark in the toolbar
while they arrive. The tabs you left open come back **asleep** — a row in the
column holding its title, loading nothing until you ask for it. Home keeps a
week of where you've been, and no longer, with private-list sites never
written down at all.

There is **no completion as you type**, and there isn't going to be: the
pages such a list offers hardest are the ones already visited most. Ctrl or
Cmd with Enter turns a bare word into its `.com`.

And there's a way back out of a stretch you didn't mean to spend: forget the
last 15 minutes, half hour, hour or two. It takes those pages, the finished
downloads, and the sites out of the wisp's memory of that stretch. How the
time felt stays — the wisp is only worth having if that part is true.

## A few more things it notices

- **Sound counts as watching.** A tab playing in front of you keeps the wisp
  awake for up to half an hour after you last touched anything, so falling
  asleep to rain sounds is rest, not screen time. Tabs you aren't looking at
  never count, however many you have open.
- **Coming back after two hours away,** the wisp greets you visibly brighter.

<br>
