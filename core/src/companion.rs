//! The companion: the one object that turns what windows report into dose
//! engine events, and the engine's state into the wisp every chrome draws.
//!
//! It sleeps between changes. Each wake-up is scheduled for the next thing
//! that can matter: a level crossed, presence lapsing, a tab turning into
//! clutter, a new day, or the next refresh of a moving wisp.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk::{gio, glib, prelude::*};

use crate::attention::{self, Signals};
use crate::bookmarks::Bookmarks;
use crate::diary;
use crate::dose::{
    self, Activity, Engine, FactorKind, Mode, Moment, Phase, Place, Rates, Snapshot, Trend,
};
use crate::feel_lab::{Lab, Step};
use crate::home::{self, Facts, PartOfDay, Topic, Words};
use crate::protocol::{
    CaptionKind, CaptionLine, DayPart, HomeAbout, HomeBookmark, HomeData, HomeExplain, HomePlace,
    HomePlant, PlantKind, ToChrome, WispMode, WispPhase, WispTrend,
};
use crate::reputation::Lists;
use crate::store::{GardenDay, Sample, Store};
use crate::window::Window;

/// How often a moving wisp is refreshed while someone is there to see it,
/// and while they aren't. The wisp eases between updates on its own, and
/// the dose moves at most 1.5% a minute, so twice a second only cost CPU.
const PRESENT_REFRESH_MS: i64 = 1_000;
const AWAY_REFRESH_MS: i64 = 5_000;

/// Coming back after this long away gets a visible welcome,
/// lasting this long.
const WELCOME_AFTER_MS: i64 = 2 * 60 * 60 * 1000;
const WELCOME_MS: i64 = 90_000;

/// Input only wakes the engine if presence is about to lapse this soon.
const LEASE_RENEW_MS: i64 = 30_000;

pub struct Companion {
    engine: RefCell<Engine>,
    seed: Lists,
    lists: RefCell<Lists>,
    store: Option<Store>,
    windows: RefCell<Vec<Weak<Window>>>,
    last_input: Cell<Option<Moment>>,
    wake: RefCell<Option<glib::SourceId>>,
    last_recorded_minute: Cell<i64>,
    last_sent: RefCell<String>,
    /// Set when playing a scripted day instead of watching the user.
    lab: RefCell<Option<Lab>>,
    lab_title: RefCell<String>,
    /// When the user went away, and until when to welcome them back.
    away_since: Cell<Option<Moment>>,
    welcome_until: Cell<Option<i64>>,
    /// When the user last came back, and how long they'd been away.
    returned: Cell<Option<(Moment, f64)>>,
    words: Words,
    bookmarks: Bookmarks,
    /// What Home remembers, when there's no store to keep it (the feel lab).
    home_memory: RefCell<HashMap<String, i64>>,
    /// Topics already noted as met, so refreshes don't write them again.
    met: RefCell<HashSet<Topic>>,
    _user_lists_monitor: RefCell<Option<gio::FileMonitor>>,
}

impl Companion {
    pub fn new(lab: Option<Lab>) -> Rc<Companion> {
        let rates = Rates::bundled();
        let now = attention::now();
        // A lab day is make-believe: it never touches the real history.
        let lab_mode = lab.is_some();
        let store = if lab_mode { None } else { open_store() };
        let engine = match store.as_ref().and_then(|s| s.latest().ok().flatten()) {
            Some(sample) => Engine::resume(
                rates,
                Snapshot {
                    dose: sample.dose,
                    at: sample.at,
                    day_load: sample.day_load,
                },
            ),
            None => Engine::new(rates, lab.as_ref().map_or(now, Lab::day_start)),
        };
        if let Some(store) = &store
            && let Err(err) = store.prune(now)
        {
            eprintln!("wisp: couldn't prune old history: {err}");
        }
        let seed = Lists::bundled();
        let lists = load_user_lists(&seed).unwrap_or_else(|| seed.clone());
        let this = Rc::new(Companion {
            engine: RefCell::new(engine),
            seed,
            lists: RefCell::new(lists),
            store,
            windows: RefCell::new(Vec::new()),
            last_input: Cell::new(None),
            wake: RefCell::new(None),
            last_recorded_minute: Cell::new(i64::MIN),
            last_sent: RefCell::new(String::new()),
            lab: RefCell::new(lab),
            lab_title: RefCell::new(String::new()),
            away_since: Cell::new(None),
            welcome_until: Cell::new(None),
            returned: Cell::new(None),
            words: Words::bundled(),
            bookmarks: if lab_mode {
                Bookmarks::in_memory()
            } else {
                open_bookmarks()
            },
            home_memory: RefCell::new(HashMap::new()),
            met: RefCell::new(HashSet::new()),
            _user_lists_monitor: RefCell::new(None),
        });
        this.watch_user_lists();
        this
    }

    pub fn add_window(self: &Rc<Self>, window: &Rc<Window>) {
        self.windows.borrow_mut().push(Rc::downgrade(window));
        self.refresh();
    }

    /// A chrome has (re)loaded and missed everything sent before: send the
    /// current state again even if nothing changed.
    pub fn chrome_ready(self: &Rc<Self>) {
        self.last_sent.borrow_mut().clear();
        self.refresh();
    }

    pub fn lab_shows_caption(&self) -> bool {
        self.lab
            .borrow()
            .as_ref()
            .is_some_and(|lab| lab.show_caption)
    }

    /// The user touched something. Cheap: only wakes the engine when this
    /// changes whether they're present.
    pub fn input(self: &Rc<Self>) {
        if self.lab.borrow().is_some() {
            return;
        }
        let now = attention::now();
        self.last_input.set(Some(now));
        let renew = match self.engine.borrow().activity() {
            Activity::Away | Activity::Listening { .. } => true,
            Activity::Present { until, .. } => until.ms - now.ms < LEASE_RENEW_MS,
        };
        if renew {
            self.refresh();
        }
    }

    /// Re-read every window's situation and bring the engine up to date.
    pub fn refresh(self: &Rc<Self>) {
        let now = attention::now();
        let windows: Vec<Rc<Window>> = {
            let mut list = self.windows.borrow_mut();
            list.retain(|w| w.strong_count() > 0);
            list.iter().filter_map(Weak::upgrade).collect()
        };
        if self.lab.borrow().is_some() {
            self.refresh_lab(now, &windows);
            return;
        }
        let front = windows.iter().find(|w| w.in_front());
        for window in &windows {
            window.mark_seen(now, front.is_some_and(|f| Rc::ptr_eq(f, window)));
        }
        let last_seen: Vec<Moment> = windows.iter().flat_map(|w| w.last_seen()).collect();

        let heard = heard_tab(&windows).map(|uri| self.lists.borrow().place(&uri));
        let activity = {
            let engine = self.engine.borrow();
            let present = front.and_then(|window| {
                let signals = Signals {
                    in_front: true,
                    last_input: self.last_input.get(),
                    sound_on_screen: window.sound_on_screen(),
                };
                attention::presence_until(engine.rates(), signals, now).map(|until| {
                    Activity::Present {
                        place: self.lists.borrow().place(&window.attended_uri()),
                        heard: heard.clone(),
                        until,
                    }
                })
            });
            // Nobody at the window: only a heard draining site still counts,
            // and only for a while after the last input.
            let listening = || {
                let heard = heard.clone().filter(is_draining)?;
                attention::listening_until(engine.rates(), self.last_input.get(), now)
                    .map(|until| Activity::Listening { heard, until })
            };
            present.or_else(listening).unwrap_or(Activity::Away)
        };
        match (&activity, self.away_since.get()) {
            (Activity::Away | Activity::Listening { .. }, None) => self.away_since.set(Some(now)),
            (Activity::Present { .. }, Some(since)) => {
                self.away_since.set(None);
                self.returned
                    .set(Some((now, (now.ms - since.ms) as f64 / 60_000.0)));
                if now.ms - since.ms >= WELCOME_AFTER_MS {
                    self.welcome_until.set(Some(now.ms + WELCOME_MS));
                }
            }
            _ => {}
        }
        {
            let mut engine = self.engine.borrow_mut();
            let clutter = attention::untouched_tabs(engine.rates(), &last_seen, now);
            engine.set_clutter(now, clutter);
            engine.set_activity(now, activity);
        }
        self.note_met(now);
        self.push(now, &windows);
        self.record(now);
        self.schedule(now, &last_seen);
    }

    // --- Home ---------------------------------------------------------

    /// Things Home explains once the user has met them.
    fn note_met(&self, now: Moment) {
        let engine = self.engine.borrow();
        let present = matches!(engine.activity(), Activity::Present { .. });
        let met = [
            (
                Topic::Draining,
                engine.mode() == Mode::Draining && engine.phase() != Phase::Rested,
            ),
            (Topic::Night, present && engine.rates().is_night(now)),
            (
                Topic::Privacy,
                matches!(
                    engine.activity(),
                    Activity::Present {
                        place: Place::Private,
                        ..
                    }
                ),
            ),
            (Topic::Heard, engine.heard_counts()),
        ];
        drop(engine);
        for (topic, happening) in met {
            if happening && self.met.borrow_mut().insert(topic) {
                let key = format!("met:{}", topic.key());
                if self.home_value(&key).is_none() {
                    self.set_home_value(&key, now.ms);
                }
            }
        }
    }

    fn home_value(&self, key: &str) -> Option<i64> {
        match &self.store {
            Some(store) => store.home_value(key).unwrap_or_else(|err| {
                eprintln!("wisp: couldn't read what Home remembers: {err}");
                None
            }),
            None => self.home_memory.borrow().get(key).copied(),
        }
    }

    fn set_home_value(&self, key: &str, value: i64) {
        match &self.store {
            Some(store) => {
                if let Err(err) = store.set_home_value(key, value) {
                    eprintln!("wisp: couldn't remember that for Home: {err}");
                }
            }
            None => {
                self.home_memory.borrow_mut().insert(key.to_owned(), value);
            }
        }
    }

    /// A button on Home, as the link it followed: `got-it/TOPIC` or
    /// `forget-bookmark/ID`. Returns whether anything changed.
    pub fn home_action(&self, action: &str) -> bool {
        match action.split_once('/') {
            Some(("got-it", topic)) => match Topic::from_key(topic) {
                Some(topic) => {
                    self.set_home_value(&format!("explained:{}", topic.key()), 1);
                    true
                }
                None => false,
            },
            Some(("forget-bookmark", id)) => match id.parse::<i64>() {
                Ok(id) => self
                    .bookmarks
                    .remove(id)
                    .map_err(|err| eprintln!("wisp: couldn't remove the bookmark: {err}"))
                    .is_ok(),
                Err(_) => false,
            },
            _ => false,
        }
    }

    pub fn is_bookmarked(&self, url: &str) -> bool {
        self.bookmarks.contains(url).unwrap_or(false)
    }

    /// Returns whether `url` is bookmarked now.
    pub fn toggle_bookmark(&self, url: &str, title: &str) -> bool {
        self.bookmarks
            .toggle(url, title, attention::now())
            .unwrap_or_else(|err| {
                eprintln!("wisp: couldn't change the bookmark: {err}");
                false
            })
    }

    /// Every open Home, in every window, shows the latest.
    pub fn refresh_homes(&self) {
        for window in self.windows.borrow().iter().filter_map(Weak::upgrade) {
            window.refresh_homes();
        }
    }

    /// Record each finished day the garden hasn't seen yet.
    fn tend_garden(&self, now: Moment) {
        let Some(store) = &self.store else { return };
        let rates = self.engine.borrow().rates().clone();
        let today = dose::day_of(&rates, now);
        let Ok(Some(first)) = store.first() else {
            return;
        };
        let recorded = store.garden().unwrap_or_default();
        let from = recorded
            .last()
            .map_or(dose::day_of(&rates, first), |d| d.day + 1)
            .max(today - 89);
        if from >= today {
            return;
        }
        let since = Moment {
            ms: now.ms - (today - from + 1) * 86_400_000,
            ..now
        };
        let days = home::days(&rates, &store.minutes_since(since).unwrap_or_default());
        for day in from..today {
            let summary = days.get(&day);
            let grown = GardenDay {
                day,
                plant: home::growth(&self.words.thresholds, summary),
                fireflies: home::fireflies(summary),
            };
            if let Err(err) = store.record_garden_day(&grown) {
                eprintln!("wisp: couldn't tend the garden: {err}");
                return;
            }
        }
    }

    /// Everything Home shows, right now.
    pub fn home_data(&self, now: Moment) -> HomeData {
        self.tend_garden(now);
        let engine = self.engine.borrow();
        let rates = engine.rates();
        let today = dose::day_of(rates, now);
        let week_ago = Moment {
            ms: now.ms - 8 * 86_400_000,
            ..now
        };
        let history_since = Moment {
            ms: now.ms - diary::DAYS * 86_400_000,
            ..now
        };
        let (days, own, garden, samples) = match &self.store {
            Some(store) => (
                home::days(rates, &store.minutes_since(week_ago).unwrap_or_default()),
                store
                    .nourishing_entries_since(Moment {
                        ms: now.ms - 30 * 86_400_000,
                        ..now
                    })
                    .unwrap_or_default(),
                store.garden().unwrap_or_default(),
                store.samples_since(history_since).unwrap_or_default(),
            ),
            None => Default::default(),
        };
        let topics = |prefix: &str| -> HashSet<Topic> {
            Topic::ALL
                .into_iter()
                .filter(|t| self.home_value(&format!("{prefix}:{}", t.key())).is_some())
                .collect()
        };
        let away_minutes = self
            .returned
            .get()
            .filter(|(at, _)| now.ms - at.ms < 10 * 60_000)
            .map_or(0.0, |(_, minutes)| minutes);
        let facts = Facts {
            now,
            dose: engine.dose(),
            met: topics("met"),
            explained: topics("explained"),
            today: days.get(&today).cloned(),
            yesterday: days.get(&(today - 1)).cloned(),
            week_nourishing: (today - 6..=today)
                .filter_map(|d| days.get(&d))
                .map(|d| d.nourishing)
                .sum(),
            away_minutes,
            garden_grew_yesterday: garden
                .last()
                .is_some_and(|d| d.day == today - 1 && d.plant.is_some()),
        };
        let greeting = home::greet(&self.words, rates, &facts);
        let part = self.words.part_of_day(now);
        let places = home::places(&self.words, &self.lists.borrow(), part, today, &own);
        let seed = self.home_value("seed").unwrap_or_else(|| {
            let seed = i64::from(glib::random_int());
            self.set_home_value("seed", seed);
            seed
        });

        let wisp = diary::build(rates, &samples, &garden, now, engine.dose());

        HomeData {
            title: greeting.title,
            line: greeting.line,
            about: greeting.about.then(|| HomeAbout {
                title: self.words.about.title.clone(),
                paragraphs: self.words.about.paragraphs.clone(),
                done: self.words.about.done.clone(),
            }),
            explain: greeting.explain.map(|(topic, text)| HomeExplain {
                topic: topic.key().to_owned(),
                text,
            }),
            part: match part {
                PartOfDay::Morning => DayPart::Morning,
                PartOfDay::Day => DayPart::Day,
                PartOfDay::Evening => DayPart::Evening,
                PartOfDay::Night => DayPart::Night,
            },
            places: places
                .into_iter()
                .map(|p| HomePlace {
                    name: p.name,
                    line: p.line,
                    url: p.url,
                    yours: p.yours,
                })
                .collect(),
            bookmarks: self
                .bookmarks
                .list()
                .unwrap_or_default()
                .into_iter()
                .map(|b| HomeBookmark {
                    id: b.id,
                    host: host_of(&b.url),
                    title: b.title,
                    url: b.url,
                })
                .collect(),
            plants: garden
                .into_iter()
                .filter_map(|d| {
                    Some(HomePlant {
                        kind: match d.plant? {
                            home::Plant::Moss => PlantKind::Moss,
                            home::Plant::Fern => PlantKind::Fern,
                            home::Plant::Flower => PlantKind::Flower,
                        },
                        fireflies: d.fireflies,
                    })
                })
                .collect(),
            seed: seed as u32,
            wisp,
        }
    }

    /// Play the scripted day up to the lab clock instead of reading windows.
    fn refresh_lab(self: &Rc<Self>, real_now: Moment, windows: &[Rc<Window>]) {
        let (lab_now, next_line) = {
            let mut lab = self.lab.borrow_mut();
            let lab = lab.as_mut().expect("checked by the caller");
            let lab_now = lab.clock(real_now);
            let mut engine = self.engine.borrow_mut();
            for step in lab.due(lab_now) {
                match step {
                    Step::Activity { at, activity } => engine.set_activity(at, activity),
                    Step::Clutter { at, tabs } => engine.set_clutter(at, tabs),
                }
            }
            engine.advance(lab_now);
            (lab_now, lab.next_line())
        };
        self.push(lab_now, windows);
        let local = (lab_now.ms / 1000 + i64::from(lab_now.utc_offset_s)).rem_euclid(86_400);
        let clock = format!("Feel lab · {:02}:{:02}", local / 3600, local / 60 % 60);
        if *self.lab_title.borrow() != clock {
            for window in windows {
                window.set_title(&clock);
            }
            self.lab_title.replace(clock);
        }

        if let Some(id) = self.wake.take() {
            id.remove();
        }
        let lab = self.lab.borrow();
        let lab = lab.as_ref().expect("checked by the caller");
        let engine = self.engine.borrow();
        let mut next = engine.next_change().ms;
        if let Some(line) = next_line {
            next = next.min(line.ms);
        }
        let delay = lab.real_delay(lab_now, next).clamp(20, PRESENT_REFRESH_MS);
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_millis(delay as u64), move || {
            if let Some(this) = weak.upgrade() {
                this.wake.take();
                this.refresh();
            }
        });
        self.wake.replace(Some(id));
    }

    fn push(&self, now: Moment, windows: &[Rc<Window>]) {
        let front = windows.iter().find(|w| w.in_front());
        let site = front
            .map(|w| host_of(&w.attended_uri()))
            .unwrap_or_default();
        let welcome = self.welcome_until.get().is_some_and(|until| now.ms < until);
        let lists = self.lists.borrow();
        let engine = self.engine.borrow();

        let heard_uri = heard_tab(windows);
        let heard = heard_uri.as_ref().map(|uri| {
            let place = lists.place(uri);
            let label = match &place {
                Place::Listed { entry, .. } => entry.clone(),
                _ => host_of(uri),
            };
            (place, label)
        });

        // The other tabs, counted but never named; private ones not even
        // counted. The heard tab has its own line.
        let mut others: Vec<(String, Moment)> = windows
            .iter()
            .flat_map(|w| w.other_tabs(front.is_some_and(|f| Rc::ptr_eq(f, w))))
            .collect();
        if let Some(uri) = &heard_uri
            && let Some(i) = others.iter().position(|(u, _)| u == uri)
        {
            others.remove(i);
        }
        others.retain(|(uri, _)| lists.place(uri) != Place::Private);
        let seen: Vec<Moment> = others.iter().map(|(_, seen)| *seen).collect();
        let untouched = attention::untouched_tabs(engine.rates(), &seen, now);
        let quiet = others.len() as u32 - untouched;

        let message = wisp_message(
            &engine,
            now,
            &site,
            heard.as_ref(),
            (quiet, untouched),
            welcome,
        );
        let json = serde_json::to_string(&message).expect("wisp messages serialise");
        if *self.last_sent.borrow() == json {
            return;
        }
        for window in windows {
            window.send_to_chrome(&message);
        }
        self.last_sent.replace(json);
    }

    fn record(&self, now: Moment) {
        let Some(store) = &self.store else { return };
        let minute = now.ms.div_euclid(60_000);
        if minute == self.last_recorded_minute.get() {
            return;
        }
        let engine = self.engine.borrow();
        let snapshot = engine.snapshot();
        let sample = Sample {
            at: now,
            dose: snapshot.dose,
            day_load: snapshot.day_load,
            mode: engine.mode(),
            place: match engine.activity() {
                Activity::Present { place, .. } => Some(place.clone()),
                Activity::Away | Activity::Listening { .. } => None,
            },
        };
        match store.record(&sample) {
            Ok(()) => self.last_recorded_minute.set(minute),
            Err(err) => eprintln!("wisp: couldn't record the dose: {err}"),
        }
    }

    fn schedule(self: &Rc<Self>, now: Moment, last_seen: &[Moment]) {
        if let Some(id) = self.wake.take() {
            id.remove();
        }
        let engine = self.engine.borrow();
        let mut next = engine.next_change().ms;
        if engine.trend() != Trend::Steady {
            let every = match engine.mode() {
                Mode::Away => AWAY_REFRESH_MS,
                _ => PRESENT_REFRESH_MS,
            };
            next = next.min(now.ms + every);
        }
        if let Some(clutter) = attention::next_untouched(engine.rates(), last_seen, now) {
            next = next.min(clutter.ms);
        }
        if let Some(until) = self.welcome_until.get().filter(|&u| u > now.ms) {
            next = next.min(until);
        }
        // Keep the minute-by-minute history going.
        next = next.min((now.ms.div_euclid(60_000) + 1) * 60_000);
        let delay = Duration::from_millis((next - now.ms).clamp(20, 60_000) as u64);
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(delay, move || {
            if let Some(this) = weak.upgrade() {
                this.wake.take();
                this.refresh();
            }
        });
        self.wake.replace(Some(id));
    }

    fn watch_user_lists(self: &Rc<Self>) {
        let file = gio::File::for_path(user_lists_path());
        let monitor =
            match file.monitor_file(gio::FileMonitorFlags::NONE, None::<&gio::Cancellable>) {
                Ok(monitor) => monitor,
                Err(err) => {
                    eprintln!("wisp: can't watch your reputation list for changes: {err}");
                    return;
                }
            };
        let weak = Rc::downgrade(self);
        monitor.connect_changed(move |_, _, _, event| {
            use gio::FileMonitorEvent as E;
            if !matches!(
                event,
                E::ChangesDoneHint | E::Created | E::Deleted | E::MovedIn
            ) {
                return;
            }
            let Some(this) = weak.upgrade() else { return };
            let lists = load_user_lists(&this.seed).unwrap_or_else(|| this.seed.clone());
            this.lists.replace(lists);
            this.refresh();
        });
        self._user_lists_monitor.replace(Some(monitor));
    }
}

/// The address of the tab heard in any window, if one is.
fn heard_tab(windows: &[Rc<Window>]) -> Option<String> {
    let front = windows.iter().find(|w| w.in_front());
    attention::heard(
        windows
            .iter()
            .flat_map(|w| w.sounds_off_screen(front.is_some_and(|f| Rc::ptr_eq(f, w)))),
    )
}

fn is_draining(place: &Place) -> bool {
    matches!(place, Place::Listed { weight, .. } if *weight < 0.0)
}

/// `https://www.example.org/a` → `example.org`; empty for local pages.
fn host_of(uri: &str) -> String {
    let Some((scheme, rest)) = uri.split_once("://") else {
        return String::new();
    };
    if !matches!(scheme, "http" | "https") {
        return String::new();
    }
    let host = rest.split(['/', '?', '#', ':']).next().unwrap_or_default();
    host.strip_prefix("www.")
        .unwrap_or(host)
        .to_ascii_lowercase()
}

fn open_store() -> Option<Store> {
    let dir = glib::user_data_dir().join("wisp");
    if let Err(err) = std::fs::create_dir_all(&dir) {
        eprintln!("wisp: can't create {}: {err}", dir.display());
        return None;
    }
    match Store::open(&dir.join("wisp.sqlite")) {
        Ok(store) => Some(store),
        Err(err) => {
            eprintln!("wisp: history is off; can't open the database: {err}");
            None
        }
    }
}

fn open_bookmarks() -> Bookmarks {
    let dir = glib::user_data_dir().join("wisp");
    let opened = std::fs::create_dir_all(&dir)
        .map_err(|err| err.to_string())
        .and_then(|()| Bookmarks::open(&dir.join("bookmarks.sqlite")).map_err(|e| e.to_string()));
    opened.unwrap_or_else(|err| {
        eprintln!("wisp: bookmarks won't be kept; can't open them: {err}");
        Bookmarks::in_memory()
    })
}

fn user_lists_path() -> PathBuf {
    glib::user_config_dir().join("wisp").join("reputation.toml")
}

/// The seed with the user's changes applied, or `None` to use the seed
/// alone (no file yet, or one with a mistake, which is reported).
fn load_user_lists(seed: &Lists) -> Option<Lists> {
    let path = user_lists_path();
    let text = std::fs::read_to_string(&path).ok()?;
    match seed.with_user(&text) {
        Ok(lists) => Some(lists),
        Err(err) => {
            eprintln!("wisp: ignoring {} until it's fixed: {err}", path.display());
            None
        }
    }
}

/// `site` is the visible tab's host and `heard` the place and label of the
/// tab heard, both named live in the caption and never stored. `others` is
/// the count of quiet and untouched other tabs.
fn wisp_message(
    engine: &Engine,
    now: Moment,
    site: &str,
    heard: Option<&(Place, String)>,
    others: (u32, u32),
    welcome: bool,
) -> ToChrome {
    let line = |kind, label: &str, heard| CaptionLine {
        kind,
        label: label.to_owned(),
        bars: 0,
        heard,
    };
    let mut now_lines = vec![match engine.activity() {
        Activity::Away | Activity::Listening { .. } => line(CaptionKind::Away, "", false),
        Activity::Present {
            place: Place::Listed { entry, weight, .. },
            ..
        } => match *weight {
            w if w < 0.0 => line(CaptionKind::Wearing, entry, false),
            w if w > 0.0 => line(CaptionKind::Restoring, entry, false),
            _ => line(CaptionKind::OrdinarySites, entry, false),
        },
        Activity::Present {
            place: Place::Unlisted,
            ..
        } => line(CaptionKind::Holding, site, false),
        Activity::Present {
            place: Place::Private,
            ..
        } => line(CaptionKind::Private, "", false),
    }];
    match heard {
        None | Some((Place::Private, _)) => {}
        Some((place, label)) => {
            let kind = match place {
                _ if !engine.heard_counts() => CaptionKind::Playing,
                Place::Listed { weight, .. } if *weight < 0.0 => CaptionKind::Wearing,
                _ => CaptionKind::Restoring,
            };
            now_lines.push(line(kind, label, true));
        }
    }
    let (quiet_tabs, untouched_tabs) = others;
    ToChrome::Wisp {
        dose: (engine.dose() * 10_000.0).round() / 10_000.0,
        phase: match engine.phase() {
            Phase::Rested => WispPhase::Rested,
            Phase::Engaged => WispPhase::Engaged,
            Phase::Clouded => WispPhase::Clouded,
            Phase::Drained => WispPhase::Drained,
        },
        mode: match engine.mode() {
            Mode::Away => WispMode::Away,
            Mode::Draining => WispMode::Draining,
            Mode::Resting => WispMode::Resting,
            Mode::Nourishing => WispMode::Nourishing,
            Mode::Holding => WispMode::Holding,
        },
        night: engine.rates().is_night(now),
        private: matches!(
            engine.activity(),
            Activity::Present {
                place: Place::Private,
                ..
            }
        ),
        welcome,
        trend: match engine.trend() {
            Trend::Rising => WispTrend::Rising,
            Trend::Falling => WispTrend::Falling,
            Trend::Steady => WispTrend::Steady,
        },
        now: now_lines,
        // Clutter is told among the other tabs instead.
        caption: engine
            .caption(now)
            .into_iter()
            .filter_map(|factor| {
                let (kind, label, heard) = match factor.what {
                    FactorKind::Wearing { entry, heard } => (CaptionKind::Wearing, entry, heard),
                    FactorKind::Restoring { entry, heard } => {
                        (CaptionKind::Restoring, entry, heard)
                    }
                    FactorKind::OrdinarySites => (CaptionKind::OrdinarySites, String::new(), false),
                    FactorKind::Away => (CaptionKind::Away, String::new(), false),
                    FactorKind::Clutter { .. } => return None,
                };
                Some(CaptionLine {
                    kind,
                    label,
                    bars: u32::from(factor.bars),
                    heard,
                })
            })
            .collect(),
        quiet_tabs,
        untouched_tabs,
    }
}
