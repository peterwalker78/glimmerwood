//! The companion: the one object that turns what windows report into dose
//! engine events, and the engine's state into the wisp every chrome draws.
//!
//! It sleeps between changes. Each wake-up is scheduled for the next thing
//! that can matter: a level crossed, presence lapsing, a new day, or the next
//! refresh of a moving wisp.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk::{gio, glib, prelude::*};

use crate::clock;
use crate::window::Window;
use glimmerwood_core::asking::{self, Asker};
use glimmerwood_core::attention::{self, Signals};
use glimmerwood_core::bookmarks::Bookmarks;
use glimmerwood_core::diary;
use glimmerwood_core::dose::{
    self, Activity, Engine, FactorKind, Mode, Moment, Phase, Place, Rates, Snapshot, Trend,
};
use glimmerwood_core::feel_lab::{Lab, Step};
use glimmerwood_core::home::{self, Facts, PartOfDay, Topic, Words};
use glimmerwood_core::places::{self, PlaceCard, Pool};
use glimmerwood_core::protocol::{
    CaptionKind, CaptionLine, DayPart, HomeAbout, HomeBookmark, HomeData, HomeExplain, HomePlace,
    HomePlant, PlantKind, Rating, SettingsData, TimeChoice, ToChrome, WispMode, WispPhase,
    WispTrend,
};
use glimmerwood_core::ratings::{self, Action};
use glimmerwood_core::reputation::{self, List, Lists};
use glimmerwood_core::settings::{self, Settings};
use glimmerwood_core::store::{GardenDay, Sample, Store};

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
    settings: RefCell<Settings>,
    /// The wisp's questions about sites it hasn't met.
    asker: RefCell<Asker>,
    last_typed: Cell<Option<Moment>>,
    /// What was last looked up on the Settings page.
    lookup: RefCell<String>,
    pool: Pool,
    /// The good places picked for the current stretch of the day, and what
    /// they were picked for (stretch, part of day, the user's own places).
    picked: RefCell<Option<(PlacesKey, Vec<PlaceCard>)>>,
    /// The country from the user's locale, for places offered only there.
    country: Option<String>,
    /// The note offering someone to talk to was closed on this visit.
    care_closed: Cell<bool>,
}

type PlacesKey = (i64, PartOfDay, Vec<String>);

impl Companion {
    pub fn new(lab: Option<Lab>) -> Rc<Companion> {
        let mut rates = Rates::bundled();
        let now = clock::now();
        // A lab day is make-believe: it never touches the real history.
        let lab_mode = lab.is_some();
        let settings = settings::load(&rates, &settings_path());
        // The lab's script is written for the usual night.
        if !lab_mode && let Err(err) = rates.set_night(&settings.night_starts, &settings.night_ends)
        {
            eprintln!("glimmerwood: keeping the usual night: {err}");
        }
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
            eprintln!("glimmerwood: couldn't prune old history: {err}");
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
            settings: RefCell::new(settings),
            asker: RefCell::new(Asker::default()),
            last_typed: Cell::new(None),
            lookup: RefCell::new(String::new()),
            pool: Pool::bundled(),
            picked: RefCell::new(None),
            country: locale_country(),
            care_closed: Cell::new(false),
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
        let now = clock::now();
        self.last_input.set(Some(now));
        let renew = match self.engine.borrow().activity() {
            Activity::Away => true,
            Activity::Present { until, .. } => until.ms - now.ms < LEASE_RENEW_MS,
        };
        if renew {
            self.refresh();
        }
    }

    /// A key was pressed: the wisp doesn't ask anything mid-sentence.
    pub fn typed(&self) {
        if self.lab.borrow().is_none() {
            self.last_typed.set(Some(clock::now()));
        }
    }

    /// Re-read every window's situation and bring the engine up to date.
    pub fn refresh(self: &Rc<Self>) {
        let now = clock::now();
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
        let activity = {
            let engine = self.engine.borrow();
            front
                .and_then(|window| {
                    let signals = Signals {
                        in_front: true,
                        last_input: self.last_input.get(),
                        sound_on_screen: window.sound_on_screen(),
                    };
                    attention::presence_until(engine.rates(), signals, now).map(|until| {
                        Activity::Present {
                            place: self.lists.borrow().place(&window.attended_uri()),
                            until,
                        }
                    })
                })
                .unwrap_or(Activity::Away)
        };
        match (&activity, self.away_since.get()) {
            (Activity::Away, None) => self.away_since.set(Some(now)),
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
            self.engine.borrow_mut().set_activity(now, activity);
        }
        let care = front.is_some_and(|w| self.lists.borrow().cares(&w.attended_uri()));
        self.note_met(now, care);
        self.push(now, &windows, care);
        self.ask(now, &windows);
        self.record(now);
        self.schedule(now);
    }

    // --- Asking about places the wisp hasn't met ---------------------------

    fn ask(&self, now: Moment, windows: &[Rc<Window>]) {
        let front = windows.iter().find(|w| w.in_front());
        let site = {
            let engine = self.engine.borrow();
            let lists = self.lists.borrow();
            match (engine.activity(), front) {
                (
                    Activity::Present {
                        place: Place::Unlisted,
                        ..
                    },
                    Some(window),
                ) => {
                    let uri = window.attended_uri();
                    // Not a part of a site carved back out of a list.
                    lists
                        .lookup(&uri)
                        .is_none()
                        .then(|| reputation::site_of(&uri, false))
                        .flatten()
                }
                _ => None,
            }
        };
        let allowed = self.settings.borrow().ask_about_new_places
            && !self.engine.borrow().rates().is_night(now);
        let day = dose::day_of(self.engine.borrow().rates(), now);
        self.asker.borrow_mut().observe(asking::Seen {
            now,
            day,
            site: site.as_deref(),
            allowed,
            last_typed: self.last_typed.get(),
        });
        self.push_question(windows);
    }

    /// The window in front shows the open question; every other window
    /// shows none.
    fn push_question(&self, windows: &[Rc<Window>]) {
        let question = self.asker.borrow().question().map(str::to_owned);
        let front = windows.iter().find(|w| w.in_front());
        for window in windows {
            let shown = front
                .is_some_and(|f| Rc::ptr_eq(f, window))
                .then(|| question.clone())
                .flatten();
            window.ask(shown);
        }
    }

    fn live_windows(&self) -> Vec<Rc<Window>> {
        self.windows
            .borrow()
            .iter()
            .filter_map(Weak::upgrade)
            .collect()
    }

    /// On a care site, the window in front quietly offers someone to talk
    /// to, until the note is closed or the user leaves the site.
    fn push_care(&self, windows: &[Rc<Window>], care: bool) {
        let present = matches!(self.engine.borrow().activity(), Activity::Present { .. });
        if !care {
            self.care_closed.set(false);
        }
        let open = care && present && !self.care_closed.get();
        let samaritans = matches!(
            self.country.as_deref(),
            Some("GB" | "IE" | "IM" | "JE" | "GG")
        );
        let front = windows.iter().find(|w| w.in_front());
        for window in windows {
            let here = front.is_some_and(|f| Rc::ptr_eq(f, window));
            window.care(open && here, samaritans);
        }
    }

    /// Closed stays closed until the user leaves the site.
    pub fn close_care(&self) {
        self.care_closed.set(true);
        for window in self.live_windows() {
            window.care(false, false);
        }
    }

    /// The question closed unanswered.
    pub fn not_now(&self, site: &str) {
        if self.asker.borrow_mut().close(site) {
            self.push_question(&self.live_windows());
        }
    }

    /// Put `site` on the list for `rating` in the user's own file, or take it
    /// out of the file to go back to Glimmerwood's rating (`None`).
    pub fn rate_site(self: &Rc<Self>, site: &str, rating: Option<Rating>) {
        let path = user_lists_path();
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let written = reputation::set_user_entry(&text, site, rating.map(ratings::list_of))
            .and_then(|text| {
                let lists = self.seed.with_user(&text)?;
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
                }
                std::fs::write(&path, &text).map_err(|e| e.to_string())?;
                Ok(lists)
            });
        match written {
            Ok(lists) => {
                self.lists.replace(lists);
                self.picked.take();
            }
            Err(err) => eprintln!("glimmerwood: couldn't rate {site}: {err}"),
        }
        self.asker.borrow_mut().close(site);
        self.refresh();
        self.refresh_pages();
    }

    // --- Settings -----------------------------------------------------------

    /// A fresh Settings page starts without an old look-up.
    pub fn forget_lookup(&self) {
        self.lookup.borrow_mut().clear();
    }

    /// Everything the Settings page shows, right now.
    pub fn settings_data(&self) -> SettingsData {
        let path = user_lists_path();
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let (entries, problem) = match reputation::user_entries(&text)
            .and_then(|entries| self.seed.with_user(&text).map(|_| entries))
        {
            Ok(entries) => (entries, String::new()),
            Err(err) => (reputation::user_entries(&text).unwrap_or_default(), err),
        };
        let yours: HashMap<String, Option<List>> = entries.iter().cloned().collect();
        let lists = self.lists.borrow();
        let rated = |site: &str| ratings::site_rating(&lists, &self.seed, &yours, site);
        let lookup = self.lookup.borrow();
        let found = reputation::site_of(&lookup, true);
        let settings = self.settings.borrow();
        let choices = |range| {
            settings::night_choices(range)
                .into_iter()
                .map(|value| TimeChoice {
                    label: clock_label(&value),
                    value,
                })
                .collect()
        };
        SettingsData {
            lookup: found
                .as_deref()
                .map(|site| {
                    let looked_up = rated(site);
                    let whole = (!looked_up.matched.is_empty() && looked_up.matched != site)
                        .then(|| rated(&looked_up.matched));
                    std::iter::once(looked_up).chain(whole).collect()
                })
                .unwrap_or_default(),
            lookup_failed: if found.is_none() {
                lookup.trim().to_owned()
            } else {
                String::new()
            },
            ratings: entries.iter().map(|(site, _)| rated(site)).collect(),
            ratings_file: tidy_path(&path),
            ratings_problem: problem,
            ask: settings.ask_about_new_places,
            night_starts: settings.night_starts.clone(),
            night_ends: settings.night_ends.clone(),
            night_start_choices: choices(settings::NIGHT_STARTS),
            night_end_choices: choices(settings::NIGHT_ENDS),
        }
    }

    /// A control on the Settings page, as the link it followed, without
    /// `glimmerwood://settings/do/`.
    pub fn settings_action(self: &Rc<Self>, action: &str) {
        let unescape =
            |text: &str| glib::Uri::unescape_string(text, None::<&str>).map(|t| t.to_string());
        let Some(action) = Action::parse(action, unescape) else {
            eprintln!("glimmerwood: Settings asked for something unknown: {action}");
            return;
        };
        match action {
            Action::Rate { site, rating } => {
                self.rate_site(&site, rating);
                return;
            }
            Action::LookUp(text) => {
                self.lookup.replace(text);
            }
            Action::Ask(on) => {
                self.settings.borrow_mut().ask_about_new_places = on;
                self.save_settings();
                self.refresh();
            }
            Action::Night { starts, ends } => {
                let moved = self.settings.borrow_mut().set_night(&starts, &ends);
                let moved = moved.and_then(|()| {
                    self.engine
                        .borrow_mut()
                        .set_night(clock::now(), &starts, &ends)
                });
                match moved {
                    Ok(()) => {
                        self.save_settings();
                        self.refresh();
                    }
                    Err(err) => eprintln!("glimmerwood: couldn't move the night: {err}"),
                }
            }
        }
        self.refresh_pages();
    }

    fn save_settings(&self) {
        if let Err(err) = settings::save(&self.settings.borrow(), &settings_path()) {
            eprintln!("glimmerwood: couldn't save the settings: {err}");
        }
    }

    // --- Home ---------------------------------------------------------

    /// Things Home explains once the user has met them.
    fn note_met(&self, now: Moment, care: bool) {
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
                !care
                    && matches!(
                        engine.activity(),
                        Activity::Present {
                            place: Place::Private,
                            ..
                        }
                    ),
            ),
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
                eprintln!("glimmerwood: couldn't read what Home remembers: {err}");
                None
            }),
            None => self.home_memory.borrow().get(key).copied(),
        }
    }

    fn set_home_value(&self, key: &str, value: i64) {
        match &self.store {
            Some(store) => {
                if let Err(err) = store.set_home_value(key, value) {
                    eprintln!("glimmerwood: couldn't remember that for Home: {err}");
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
                    .map_err(|err| eprintln!("glimmerwood: couldn't remove the bookmark: {err}"))
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
            .toggle(url, title, clock::now())
            .unwrap_or_else(|err| {
                eprintln!("glimmerwood: couldn't change the bookmark: {err}");
                false
            })
    }

    /// Every open Home and Settings page, in every window, shows the latest.
    pub fn refresh_pages(&self) {
        for window in self.live_windows() {
            window.refresh_pages();
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
                eprintln!("glimmerwood: couldn't tend the garden: {err}");
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
        let seed = self.home_value("seed").unwrap_or_else(|| {
            let seed = i64::from(glib::random_int());
            self.set_home_value("seed", seed);
            seed
        });
        let places = self.good_places(now, part, engine.phase(), seed as u32, &own);

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

    /// Home's good places, picked once for each stretch of the day. What was
    /// offered is remembered, so the next stretch offers something else.
    fn good_places(
        &self,
        now: Moment,
        part: PartOfDay,
        phase: Phase,
        seed: u32,
        own: &[(String, f64)],
    ) -> Vec<PlaceCard> {
        let local_s = now.ms.div_euclid(1000) + i64::from(now.utc_offset_s);
        let stretch = self.pool.stretch(local_s);
        let mut own_entries: Vec<String> = own.iter().map(|(entry, _)| entry.clone()).collect();
        own_entries.sort();
        let key = (stretch, part, own_entries);
        if let Some((picked_for, cards)) = self.picked.borrow().as_ref()
            && *picked_for == key
        {
            return cards.clone();
        }
        let offered: HashMap<String, i64> = match &self.store {
            Some(store) => store
                .home_values_with_prefix(OFFERED)
                .unwrap_or_default()
                .into_iter()
                .collect(),
            None => self
                .home_memory
                .borrow()
                .iter()
                .filter_map(|(k, v)| Some((k.strip_prefix(OFFERED)?.to_owned(), *v)))
                .collect(),
        };
        let at = places::Moment {
            part,
            month: places::month_of(local_s),
            stretch,
            phase,
            country: self.country.as_deref(),
            seed,
        };
        let cards = places::pick(
            &self.pool,
            &self.lists.borrow(),
            at,
            own,
            self.words.thresholds.own_place_minutes,
            &offered,
        );
        for card in cards.iter().filter(|c| !c.yours) {
            if offered.get(&card.entry) != Some(&stretch) {
                self.set_home_value(&format!("{OFFERED}{}", card.entry), stretch);
            }
        }
        self.picked.replace(Some((key, cards.clone())));
        cards
    }

    /// Play the scripted day up to the lab clock instead of reading windows.
    fn refresh_lab(self: &Rc<Self>, real_now: Moment, windows: &[Rc<Window>]) {
        let (lab_now, next_line) = {
            let mut lab = self.lab.borrow_mut();
            let lab = lab.as_mut().expect("checked by the caller");
            let lab_now = lab.clock(real_now);
            let mut engine = self.engine.borrow_mut();
            for Step { at, activity } in lab.due(lab_now) {
                engine.set_activity(at, activity);
            }
            engine.advance(lab_now);
            (lab_now, lab.next_line())
        };
        self.push(lab_now, windows, false);
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

    fn push(&self, now: Moment, windows: &[Rc<Window>], care: bool) {
        let front = windows.iter().find(|w| w.in_front());
        let site = front
            .map(|w| host_of(&w.attended_uri()))
            .unwrap_or_default();
        let welcome = self.welcome_until.get().is_some_and(|until| now.ms < until);
        let lists = self.lists.borrow();
        let engine = self.engine.borrow();

        // The other tabs, counted but never named; private ones not even
        // counted.
        let other_tabs = windows
            .iter()
            .flat_map(|w| w.other_tabs(front.is_some_and(|f| Rc::ptr_eq(f, w))))
            .filter(|uri| lists.place(uri) != Place::Private)
            .count() as u32;

        let message = wisp_message(&engine, now, &site, other_tabs, welcome, care);
        self.push_care(windows, care);
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
                Activity::Away => None,
            },
        };
        match store.record(&sample) {
            Ok(()) => self.last_recorded_minute.set(minute),
            Err(err) => eprintln!("glimmerwood: couldn't record the dose: {err}"),
        }
    }

    fn schedule(self: &Rc<Self>, now: Moment) {
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
        if let Some(until) = self.welcome_until.get().filter(|&u| u > now.ms) {
            next = next.min(until);
        }
        if let Some(due) = self.asker.borrow().due() {
            next = next.min(due.ms);
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
                    eprintln!("glimmerwood: can't watch your reputation list for changes: {err}");
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
            this.picked.take();
            this.refresh();
            this.refresh_pages();
        });
        self._user_lists_monitor.replace(Some(monitor));
    }
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
    let dir = glib::user_data_dir().join("glimmerwood");
    if let Err(err) = std::fs::create_dir_all(&dir) {
        eprintln!("glimmerwood: can't create {}: {err}", dir.display());
        return None;
    }
    match Store::open(&dir.join("glimmerwood.sqlite")) {
        Ok(store) => Some(store),
        Err(err) => {
            eprintln!("glimmerwood: history is off; can't open the database: {err}");
            None
        }
    }
}

fn open_bookmarks() -> Bookmarks {
    let dir = glib::user_data_dir().join("glimmerwood");
    let opened = std::fs::create_dir_all(&dir)
        .map_err(|err| err.to_string())
        .and_then(|()| Bookmarks::open(&dir.join("bookmarks.sqlite")).map_err(|e| e.to_string()));
    opened.unwrap_or_else(|err| {
        eprintln!("glimmerwood: bookmarks won't be kept; can't open them: {err}");
        Bookmarks::in_memory()
    })
}

/// What Home remembers about the good places it offered.
const OFFERED: &str = "offered:";

/// `en_GB.UTF-8` → `GB`: the country of the first language the user set.
fn locale_country() -> Option<String> {
    glib::language_names().iter().find_map(|name| {
        let (_, rest) = name.split_once('_')?;
        let country: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphabetic())
            .collect();
        (country.len() == 2).then(|| country.to_ascii_uppercase())
    })
}

/// A path with the home directory written as `~`.
fn tidy_path(path: &std::path::Path) -> String {
    let home = glib::home_dir();
    match path.strip_prefix(&home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// "23:00" → "11pm", "00:30" → "12:30am".
fn clock_label(time: &str) -> String {
    let (h, m) = time.split_once(':').unwrap_or((time, "00"));
    let h: u32 = h.parse().unwrap_or(0);
    if h == 0 && m == "00" {
        return "midnight".into();
    }
    let (hour, half) = (
        if h.is_multiple_of(12) { 12 } else { h % 12 },
        if h < 12 { "am" } else { "pm" },
    );
    if m == "00" {
        format!("{hour}{half}")
    } else {
        format!("{hour}:{m}{half}")
    }
}

fn settings_path() -> PathBuf {
    glib::user_config_dir()
        .join("glimmerwood")
        .join("settings.toml")
}

fn user_lists_path() -> PathBuf {
    glib::user_config_dir()
        .join("glimmerwood")
        .join("reputation.toml")
}

/// The seed with the user's changes applied, or `None` to use the seed
/// alone (no file yet, or one with a mistake, which is reported).
fn load_user_lists(seed: &Lists) -> Option<Lists> {
    let path = user_lists_path();
    let text = std::fs::read_to_string(&path).ok()?;
    match seed.with_user(&text) {
        Ok(lists) => Some(lists),
        Err(err) => {
            eprintln!(
                "glimmerwood: ignoring {} until it's fixed: {err}",
                path.display()
            );
            None
        }
    }
}

/// `site` is the visible tab's host, named live in the caption and never
/// stored. `other_tabs` counts the tabs that aren't on screen.
fn wisp_message(
    engine: &Engine,
    now: Moment,
    site: &str,
    other_tabs: u32,
    welcome: bool,
    care: bool,
) -> ToChrome {
    let line = |kind, label: &str| CaptionLine {
        kind,
        label: label.to_owned(),
        bars: 0,
    };
    let now_line = match engine.activity() {
        Activity::Away => line(CaptionKind::Away, ""),
        Activity::Present {
            place: Place::Listed { entry, weight, .. },
            ..
        } => match *weight {
            w if w < 0.0 => line(CaptionKind::Wearing, entry),
            w if w > 0.0 => line(CaptionKind::Restoring, entry),
            _ => line(CaptionKind::OrdinarySites, entry),
        },
        Activity::Present {
            place: Place::Unlisted,
            ..
        } => line(CaptionKind::Holding, site),
        Activity::Present {
            place: Place::Private,
            ..
        } if care => line(CaptionKind::Care, ""),
        Activity::Present {
            place: Place::Private,
            ..
        } => line(CaptionKind::Private, ""),
    };
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
        // On a care site the wisp stays close rather than turning away.
        private: !care
            && matches!(
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
        now: now_line,
        caption: engine
            .caption(now)
            .into_iter()
            .map(|factor| {
                let (kind, label) = match factor.what {
                    FactorKind::Wearing { entry } => (CaptionKind::Wearing, entry),
                    FactorKind::Restoring { entry } => (CaptionKind::Restoring, entry),
                    FactorKind::OrdinarySites => (CaptionKind::OrdinarySites, String::new()),
                    FactorKind::Away => (CaptionKind::Away, String::new()),
                };
                CaptionLine {
                    kind,
                    label,
                    bars: u32::from(factor.bars),
                }
            })
            .collect(),
        other_tabs,
    }
}
