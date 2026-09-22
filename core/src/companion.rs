//! The companion: the one object that turns what windows report into dose
//! engine events, and the engine's state into the wisp every chrome draws.
//!
//! It sleeps between changes. Each wake-up is scheduled for the next thing
//! that can matter: a level crossed, presence lapsing, a new day, or the next
//! refresh of a moving wisp.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::asking::{self, Asker};
use crate::attention::{self, Signals};
use crate::bookmarks::Bookmarks;
use crate::diary;
use crate::dose::{
    self, Activity, Engine, FactorKind, Mode, Moment, Phase, Place, Rates, Snapshot, Trend,
};
use crate::downloads::{Download, Downloads, Progress};
use crate::feel_lab::{Lab, Step};
use crate::history::History;
use crate::home::{self, Facts, PartOfDay, Topic, Words};
use crate::host::{Host, Window};
use crate::nav;
use crate::nav::host_of;
use crate::newsboat::{self, GoodNews};
use crate::places::{self, PlaceCard, Pool};
use crate::protocol::Page;
use crate::protocol::{
    CaptionKind, CaptionLine, DayPart, HomeAbout, HomeBookmark, HomeData, HomeExplain, HomePlace,
    HomePlant, NewsFeed, PlantKind, Rating, SettingsData, TimeChoice, ToChrome, WispMode,
    WispPhase, WispTrend,
};
use crate::ratings::{self, Action};
use crate::reputation::{self, List, Lists};
use crate::session::Session;
use crate::settings::{self, Settings};
use crate::store::{GardenDay, Sample, Store};

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
    /// Pages visited, kept a week. Its own database: the dose store never
    /// sees an address, and this one holds nothing else.
    history: Option<History>,
    /// What is arriving now, and what arrived and hasn't been opened.
    downloads: RefCell<Downloads>,
    host: Rc<dyn Host>,
    last_input: Cell<Option<Moment>>,
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
    settings: RefCell<Settings>,
    /// The wisp's questions about sites it hasn't met.
    asker: RefCell<Asker>,
    last_typed: Cell<Option<Moment>>,
    /// What was last looked up on the Settings page.
    lookup: RefCell<String>,
    /// Feeds for Newsboat, and what the last press of its button did.
    good_news: GoodNews,
    newsboat_done: RefCell<String>,
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
    pub fn new(host: Rc<dyn Host>, lab: Option<Lab>) -> Rc<Companion> {
        let mut rates = Rates::bundled();
        let now = host.now();
        // A lab day is make-believe: it never touches the real history.
        let lab_mode = lab.is_some();
        let settings = settings::load(&rates, &settings_path(&host));
        // The lab's script is written for the usual night.
        if !lab_mode && let Err(err) = rates.set_night(&settings.night_starts, &settings.night_ends)
        {
            eprintln!("glimmerwood: keeping the usual night: {err}");
        }
        let store = if lab_mode {
            None
        } else {
            open_store(&host.data_dir())
        };
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
        let history = if lab_mode {
            None
        } else {
            open_history(&host.data_dir())
        };
        if let Some(history) = &history
            && let Err(err) = history.prune(now)
        {
            eprintln!("glimmerwood: couldn't drop pages older than the week: {err}");
        }
        let seed = Lists::bundled();
        let lists = load_user_lists(&seed, &user_lists_path(&host)).unwrap_or_else(|| seed.clone());
        Rc::new(Companion {
            engine: RefCell::new(engine),
            seed,
            lists: RefCell::new(lists),
            store,
            history,
            downloads: RefCell::new(Downloads::new()),
            host: host.clone(),
            last_input: Cell::new(None),
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
                open_bookmarks(&host.data_dir())
            },
            home_memory: RefCell::new(HashMap::new()),
            met: RefCell::new(HashSet::new()),
            settings: RefCell::new(settings),
            asker: RefCell::new(Asker::default()),
            last_typed: Cell::new(None),
            lookup: RefCell::new(String::new()),
            good_news: GoodNews::bundled(),
            newsboat_done: RefCell::new(String::new()),
            pool: Pool::bundled(),
            picked: RefCell::new(None),
            country: host.locale_country(),
            care_closed: Cell::new(false),
        })
    }

    /// The moment it is now, as the host reckons it.
    pub fn now(&self) -> Moment {
        self.host.now()
    }

    /// A window has opened or closed; the host knows which are live.
    pub fn windows_changed(self: &Rc<Self>) {
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
        let now = self.host.now();
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
            self.last_typed.set(Some(self.host.now()));
        }
    }

    /// Re-read every window's situation and bring the engine up to date.
    pub fn refresh(self: &Rc<Self>) {
        let now = self.host.now();
        let windows = self.host.windows();
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

    fn ask(&self, now: Moment, windows: &[Rc<dyn Window>]) {
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
    fn push_question(&self, windows: &[Rc<dyn Window>]) {
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

    fn live_windows(&self) -> Vec<Rc<dyn Window>> {
        self.host.windows()
    }

    /// On a care site, the window in front quietly offers someone to talk
    /// to, until the note is closed or the user leaves the site.
    fn push_care(&self, windows: &[Rc<dyn Window>], care: bool) {
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
        let path = user_lists_path(&self.host);
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

    /// A fresh Settings page starts without an old look-up, or word of
    /// what the good news button last did.
    pub fn forget_lookup(&self) {
        self.lookup.borrow_mut().clear();
        self.newsboat_done.borrow_mut().clear();
    }

    /// Everything the Settings page shows, right now.
    pub fn settings_data(&self) -> SettingsData {
        let path = user_lists_path(&self.host);
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
        let home = self.host.home_dir();
        let newsboat_file = newsboat::urls_file(&home, Path::is_dir);
        let feeds_text = newsboat_file
            .as_deref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .unwrap_or_default();
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
            ratings_file: tidy_path(&path, &home),
            ratings_problem: problem,
            ask: settings.ask_about_new_places,
            night_starts: settings.night_starts.clone(),
            night_ends: settings.night_ends.clone(),
            night_start_choices: choices(settings::NIGHT_STARTS),
            night_end_choices: choices(settings::NIGHT_ENDS),
            newsboat_file: newsboat_file
                .as_deref()
                .map(|path| tidy_path(path, &home))
                .unwrap_or_default(),
            good_news: self
                .good_news
                .feeds
                .iter()
                .zip(newsboat::present(&feeds_text, &self.good_news.feeds))
                .map(|(feed, added)| NewsFeed {
                    name: feed.name.clone(),
                    site: feed.site.clone(),
                    kind: feed.kind,
                    added,
                })
                .collect(),
            newsboat_done: self.newsboat_done.borrow().clone(),
        }
    }

    /// Put the good news feeds into Newsboat's list, or take them out, and
    /// say what happened.
    fn send_to_newsboat(&self, add: bool) {
        let home = self.host.home_dir();
        let Some(path) = newsboat::urls_file(&home, Path::is_dir) else {
            self.newsboat_done
                .replace("Newsboat hasn't been run on this computer yet.".into());
            return;
        };
        let shown = tidy_path(&path, &home);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(err) => {
                self.newsboat_done
                    .replace(format!("Couldn't read {shown}: {err}"));
                return;
            }
        };
        let feeds = &self.good_news.feeds;
        let (text, done) = if add {
            let added = newsboat::add(&text, feeds);
            let done = match (added.added, added.already) {
                (0, _) => format!("{shown} has every one of them already."),
                (n, 0) => format!("Added {} to {shown}. {READ_THEM}", feeds_counted(n)),
                (n, had) => format!(
                    "Added {} to {shown}; it had {had} already. {READ_THEM}",
                    feeds_counted(n)
                ),
            };
            (added.text, done)
        } else {
            let (text, removed) = newsboat::remove(&text, feeds);
            let done = match removed {
                0 => format!("There was nothing of Glimmerwood's in {shown} to take out."),
                n => format!(
                    "Took {} out of {shown}. Your own feeds are as they were.",
                    feeds_counted(n)
                ),
            };
            (text, done)
        };
        let done = match std::fs::write(&path, text) {
            Ok(()) => done,
            Err(err) => format!("Couldn't write {shown}: {err}"),
        };
        self.newsboat_done.replace(done);
    }

    /// A control on the Settings page, as the link it followed, without
    /// `glimmerwood://settings/do/`.
    pub fn settings_action(self: &Rc<Self>, action: &str) {
        let unescape = |text: &str| Some(nav::unescape(text));
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
                        .set_night(self.host.now(), &starts, &ends)
                });
                match moved {
                    Ok(()) => {
                        self.save_settings();
                        self.refresh();
                    }
                    Err(err) => eprintln!("glimmerwood: couldn't move the night: {err}"),
                }
            }
            Action::Newsboat { add } => self.send_to_newsboat(add),
        }
        self.refresh_pages();
    }

    fn save_settings(&self) {
        if let Err(err) = settings::save(&self.settings.borrow(), &settings_path(&self.host)) {
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
    pub fn home_action(self: &Rc<Self>, action: &str) -> bool {
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
            Some(("open-download", id)) => match id.parse::<u32>() {
                Ok(id) => match self.open_download(id) {
                    Some(path) => self.host.open_file(&path),
                    None => false,
                },
                Err(_) => false,
            },
            // A way out of somewhere you didn't mean to go: the pages, the
            // finished downloads, and the sites out of the wisp's own memory
            // of that stretch. What it never touches is how the time felt —
            // the wisp is only worth having if it can't be talked round.
            Some(("forget-since", minutes)) => {
                let Ok(minutes) = minutes.parse::<i64>() else {
                    return false;
                };
                if !(1..=MOST_MINUTES_FORGOTTEN).contains(&minutes) {
                    return false;
                }
                let now = self.now();
                let ms = minutes * 60_000;
                if let Some(history) = &self.history
                    && let Err(err) = history.forget_since(now, ms)
                {
                    eprintln!("glimmerwood: couldn't forget those pages: {err}");
                    return false;
                }
                if let Some(store) = &self.store
                    && let Err(err) = store.forget_entries_since(now, ms)
                {
                    eprintln!("glimmerwood: couldn't forget those sites: {err}");
                    return false;
                }
                self.downloads.borrow_mut().forget_finished();
                true
            }
            _ => false,
        }
    }

    // --- Pages visited, downloads, and the session ----------------------------

    /// Remember a page, unless it is on the private list, where nothing is
    /// written down at all.
    pub fn visited(&self, url: &str, title: &str) {
        // A private-list site is never written down, here or anywhere.
        if matches!(self.lists.borrow().place(url), Place::Private) {
            return;
        }
        let Some(history) = &self.history else { return };
        if let Err(err) = history.record(self.now(), url, title) {
            eprintln!("glimmerwood: couldn't remember a page: {err}");
        }
    }

    /// This week, newest first.
    pub fn week_of_pages(&self) -> Vec<Page> {
        let Some(history) = &self.history else {
            return Vec::new();
        };
        history
            .week(self.now())
            .unwrap_or_default()
            .into_iter()
            .map(|visit| Page {
                at: visit.at,
                url: visit.url,
                title: visit.title,
                host: visit.host,
            })
            .collect()
    }

    /// Where the open tabs are kept, for the shell that writes them.
    pub fn session_path(&self) -> PathBuf {
        self.host
            .data_dir()
            .join("glimmerwood")
            .join("session.json")
    }

    pub fn last_session(&self) -> Session {
        Session::load(&self.session_path()).worth_restoring()
    }

    pub fn keep_session(&self, session: &Session) {
        if let Err(err) = session.save(&self.session_path()) {
            eprintln!("glimmerwood: couldn't keep the open tabs: {err}");
        }
    }

    pub fn download_started(self: &Rc<Self>, name: &str, path: &str) -> u32 {
        let id = self.downloads.borrow_mut().started(name, path);
        self.push_downloads();
        self.refresh_pages();
        id
    }

    pub fn download_progressed(self: &Rc<Self>, id: u32, fraction: Option<f64>) {
        if self.downloads.borrow_mut().progressed(id, fraction) {
            self.refresh_pages();
        }
    }

    pub fn download_finished(self: &Rc<Self>, id: u32, progress: Progress, path: Option<&str>) {
        if self.downloads.borrow_mut().finished(id, progress, path) {
            self.push_downloads();
            self.refresh_pages();
        }
    }

    /// The file to open, if it is still listed. Opening it takes it off Home.
    pub fn open_download(self: &Rc<Self>, id: u32) -> Option<String> {
        let path = self.downloads.borrow_mut().opened(id);
        if path.is_some() {
            self.push_downloads();
            self.refresh_pages();
        }
        path
    }

    pub fn downloads_showing(&self) -> Vec<Download> {
        self.downloads.borrow().showing()
    }

    fn push_downloads(self: &Rc<Self>) {
        let running = self.downloads.borrow().running() as u32;
        for window in self.host.windows() {
            window.send_to_chrome(&ToChrome::Downloads { running });
        }
    }

    pub fn is_bookmarked(&self, url: &str) -> bool {
        self.bookmarks.contains(url).unwrap_or(false)
    }

    /// Returns whether `url` is bookmarked now.
    pub fn toggle_bookmark(&self, url: &str, title: &str) -> bool {
        self.bookmarks
            .toggle(url, title, self.host.now())
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
            let seed = i64::from(self.host.noise());
            self.set_home_value("seed", seed);
            seed
        });
        let places = self.good_places(now, part, engine.phase(), seed as u32, &own);

        let wisp = diary::build(rates, &samples, &garden, now, engine.dose());

        HomeData {
            pages: self.week_of_pages(),
            downloads: self.downloads_showing(),
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
    fn refresh_lab(self: &Rc<Self>, real_now: Moment, windows: &[Rc<dyn Window>]) {
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

        let lab = self.lab.borrow();
        let lab = lab.as_ref().expect("checked by the caller");
        let engine = self.engine.borrow();
        let mut next = engine.next_change().ms;
        if let Some(line) = next_line {
            next = next.min(line.ms);
        }
        let delay = lab.real_delay(lab_now, next).clamp(20, PRESENT_REFRESH_MS);
        self.host.wake_in(delay as u64);
    }

    fn push(&self, now: Moment, windows: &[Rc<dyn Window>], care: bool) {
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
        self.host.wake_in((next - now.ms).clamp(20, 60_000) as u64);
    }

    /// The user's own list file has changed underneath us. Whoever is
    /// watching it says so; the companion does not watch files itself.
    pub fn user_lists_changed(self: &Rc<Self>) {
        let path = user_lists_path(&self.host);
        let lists = load_user_lists(&self.seed, &path).unwrap_or_else(|| self.seed.clone());
        self.lists.replace(lists);
        self.picked.take();
        self.refresh();
        self.refresh_pages();
    }
}

fn open_store(data: &std::path::Path) -> Option<Store> {
    let dir = data.join("glimmerwood");
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

fn open_history(data: &std::path::Path) -> Option<History> {
    let dir = data.join("glimmerwood");
    if let Err(err) = std::fs::create_dir_all(&dir) {
        eprintln!("glimmerwood: can't create {}: {err}", dir.display());
        return None;
    }
    match History::open(&dir.join("pages.sqlite")) {
        Ok(history) => Some(history),
        Err(err) => {
            eprintln!("glimmerwood: pages won't be remembered: {err}");
            None
        }
    }
}

fn open_bookmarks(data: &std::path::Path) -> Bookmarks {
    let dir = data.join("glimmerwood");
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

/// The longest stretch that can be forgotten in one go. Beyond a couple of
/// hours it stops being "that wasn't the afternoon I meant to have" and
/// becomes a way to keep no history at all, which is what the week is for.
const MOST_MINUTES_FORGOTTEN: i64 = 120;

/// `en_GB.UTF-8` → `GB`: the country of the first language the user set.
/// Every platform names its locales this way; only the asking differs.
pub fn country_from_locales(names: &[String]) -> Option<String> {
    names.iter().find_map(|name| {
        let (_, rest) = name.split_once('_')?;
        let country: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphabetic())
            .collect();
        (country.len() == 2).then(|| country.to_ascii_uppercase())
    })
}

/// A path with the home directory written as `~`.
fn tidy_path(path: &std::path::Path, home: &std::path::Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// Said after good news goes into Newsboat's list: it fetches nothing until
/// asked, and `R` fetches every feed.
const READ_THEM: &str = "To read them, run newsboat in a terminal and press Shift+R.";

/// "1 feed", "12 feeds".
fn feeds_counted(n: usize) -> String {
    if n == 1 {
        "1 feed".into()
    } else {
        format!("{n} feeds")
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

fn settings_path(host: &Rc<dyn Host>) -> PathBuf {
    host.config_dir().join("glimmerwood").join("settings.toml")
}

fn user_lists_path(host: &Rc<dyn Host>) -> PathBuf {
    host.config_dir()
        .join("glimmerwood")
        .join("reputation.toml")
}

/// The seed with the user's changes applied, or `None` to use the seed
/// alone (no file yet, or one with a mistake, which is reported).
fn load_user_lists(seed: &Lists, path: &std::path::Path) -> Option<Lists> {
    let text = std::fs::read_to_string(path).ok()?;
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
