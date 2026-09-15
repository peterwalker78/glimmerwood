//! A browser window: the chrome layered over a stack of tabs.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use gtk::{gdk, gio, glib, prelude::*};
use webkit::prelude::*;

use crate::attention;
use crate::companion::Companion;
use crate::dose::Mode;
use crate::failure::{self, Reason};
use crate::protocol::{ChromeView, Security, TabInfo, TabSound, ToChrome, ToCore, WispMode};
use crate::wisp_view::WispView;
use crate::{chrome, find, nav, prefs, scheme, tabs};

/// The toolbar's height until it reports its own.
const INITIAL_TOOLBAR_HEIGHT: i32 = 65;
/// How long after the tab column stops moving its width is saved.
const SAVE_WIDTH_AFTER_MS: u64 = 600;

/// Tabs reachable with Ctrl+number; Ctrl+9 is always the last tab.
const NUMBERED_TABS: u32 = 8;

pub fn install_accels(app: &gtk::Application) {
    app.set_accels_for_action("win.focus-address", &["<Control>l", "<Alt>d", "F6"]);
    app.set_accels_for_action("win.back", &["<Alt>Left"]);
    app.set_accels_for_action("win.forward", &["<Alt>Right"]);
    app.set_accels_for_action("win.reload", &["<Control>r", "F5"]);
    app.set_accels_for_action("win.new-tab", &["<Control>t"]);
    app.set_accels_for_action("win.close-tab", &["<Control>w", "<Control>F4"]);
    app.set_accels_for_action("win.bookmark", &["<Control>d"]);
    app.set_accels_for_action("win.home", &["<Alt>Home"]);
    app.set_accels_for_action("win.find", &["<Control>f"]);
    app.set_accels_for_action("win.settings", &["<Control>comma"]);
    app.set_accels_for_action("win.find-next", &["<Control>g", "F3"]);
    app.set_accels_for_action("win.find-previous", &["<Control><Shift>g", "<Shift>F3"]);
    app.set_accels_for_action("win.next-tab", &["<Control>Tab", "<Control>Page_Down"]);
    app.set_accels_for_action(
        "win.previous-tab",
        &[
            "<Control><Shift>Tab",
            "<Control><Shift>ISO_Left_Tab",
            "<Control>Page_Up",
        ],
    );
    for n in 1..=NUMBERED_TABS {
        app.set_accels_for_action(&format!("win.tab-{n}"), &[&format!("<Control>{n}")]);
    }
    app.set_accels_for_action("win.last-tab", &["<Control>9"]);
}

/// A pointer press: device, button, window coordinates and time.
type Press = (gdk::Device, u32, f64, f64, u32);

/// What a window action does when triggered.
type Action = Box<dyn Fn(&Rc<Window>)>;

struct Tab {
    id: u32,
    view: webkit::WebView,
    /// While an upgraded HTTPS attempt is in flight: the address it tried and
    /// the plain HTTP address to use if that exact attempt can't connect.
    fallback: RefCell<Option<nav::Target>>,
    /// The address whose failure page this tab is showing, if it is. Time on
    /// a failure page is time on no site at all.
    failed: RefCell<Option<String>>,
    /// A failure page was asked for and hasn't started loading yet.
    failure_pending: Cell<bool>,
    /// The site icon as a `data:` URL, encoded once when it changes.
    icon: RefCell<Option<String>>,
}

pub struct Window {
    companion: Rc<Companion>,
    window: gtk::ApplicationWindow,
    paned: gtk::Paned,
    stack: gtk::Stack,
    sidebar: RefCell<Option<webkit::WebView>>,
    toolbar: RefCell<Option<webkit::WebView>>,
    wisp: RefCell<Option<WispView>>,
    /// The last press on the window, for dragging it from empty chrome.
    last_press: RefCell<Option<Press>>,
    tabs: RefCell<Vec<Rc<Tab>>>,
    selected: Cell<u32>,
    next_id: Cell<u32>,
    state_queued: Cell<bool>,
    /// How much of the window's top the toolbar webview covers: the bar, or
    /// more while the caption is open. It is transparent below the bar but
    /// still takes clicks there, so it must never be taller than this.
    toolbar_cover: Cell<i32>,
    /// What each chrome page was last sent that it would redraw for, so an
    /// update that changes nothing it shows isn't sent at all.
    sent_tabs: RefCell<String>,
    sent_wisp: RefCell<String>,
    /// The site the wisp's question was last sent about, if any.
    sent_ask: RefCell<Option<Option<String>>>,
    /// Whether the note offering someone to talk to was last sent open.
    sent_care: Cell<Option<(bool, bool)>>,
    /// The find bar is open, and what it last searched for.
    finding: Cell<bool>,
    find_query: RefCell<String>,
    /// Searches still waiting for their count. Only the latest search's count
    /// is shown; moving to the next match reports a count of one, which
    /// isn't news.
    find_pending: Cell<u32>,
}

impl Window {
    pub fn new(app: &gtk::Application, companion: &Rc<Companion>) -> Rc<Self> {
        // The tab column and the page side by side, below the toolbar. The
        // toolbar floats over the top so the wisp's caption can open over the
        // page; the column's edge is a native handle the user drags.
        let stack = gtk::Stack::new();
        let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
        paned.set_margin_top(INITIAL_TOOLBAR_HEIGHT);
        paned.set_resize_start_child(false);
        paned.set_shrink_start_child(false);
        paned.set_end_child(Some(&stack));
        paned.set_position(prefs::tab_column_width());
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&paned));

        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title("Glimmerwood")
            .default_width(1200)
            .default_height(800)
            .child(&overlay)
            .build();
        // No titlebar: a hidden one keeps GTK's resize edges and
        // shadow but takes no height. The chrome drags the window instead.
        let titlebar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        titlebar.set_visible(false);
        window.set_titlebar(Some(&titlebar));

        let this = Rc::new(Self {
            companion: Rc::clone(companion),
            window,
            paned: paned.clone(),
            stack: stack.clone(),
            sidebar: RefCell::new(None),
            toolbar: RefCell::new(None),
            wisp: RefCell::new(None),
            last_press: RefCell::new(None),
            tabs: RefCell::new(Vec::new()),
            selected: Cell::new(0),
            next_id: Cell::new(1),
            state_queued: Cell::new(false),
            toolbar_cover: Cell::new(INITIAL_TOOLBAR_HEIGHT),
            sent_tabs: RefCell::new(String::new()),
            sent_wisp: RefCell::new(String::new()),
            sent_ask: RefCell::new(None),
            sent_care: Cell::new(None),
            finding: Cell::new(false),
            find_query: RefCell::new(String::new()),
            find_pending: Cell::new(0),
        });

        let weak = Rc::downgrade(&this);
        let (toolbar, sidebar) = chrome::new_pair(move |message| {
            if let Some(this) = weak.upgrade() {
                this.handle(message);
            }
        });
        overlay.add_overlay(&toolbar);
        // Placed by hand rather than by measuring: a WebKitWebView reports its
        // current height as its natural height, so it never shrinks back after
        // the caption closes and goes on swallowing clicks on the tabs below.
        let weak = Rc::downgrade(&this);
        let placed = toolbar.clone();
        overlay.connect_get_child_position(move |overlay, child| {
            let this = weak.upgrade()?;
            (child == placed.upcast_ref::<gtk::Widget>()).then(|| {
                // GTK expects every child to be measured before it's placed.
                let _ = child.measure(gtk::Orientation::Vertical, overlay.width());
                gdk::Rectangle::new(0, 0, overlay.width(), this.toolbar_cover.get())
            })
        });
        this.toolbar.replace(Some(toolbar));
        sidebar.set_width_request(prefs::MIN_TAB_COLUMN_WIDTH);
        paned.set_start_child(Some(&sidebar));
        this.sidebar.replace(Some(sidebar));
        this.remember_tab_column_width();

        let weak = Rc::downgrade(&this);
        let weak_click = weak.clone();
        let wisp = WispView::new(
            move |open| {
                if let Some(this) = weak.upgrade() {
                    this.send_to_chrome(&ToChrome::Caption { open });
                }
            },
            move || {
                if let Some(this) = weak_click.upgrade() {
                    this.show_wisp();
                }
            },
        );
        overlay.add_overlay(wisp.widget());
        this.wisp.replace(Some(wisp));

        this.install_actions();
        this.watch_attention();
        this.add_home_tab();

        // The GTK window owns this struct. Every other closure holds a weak
        // reference; this handler holds the strong one, and GTK drops it when
        // the window is destroyed.
        let owner = Rc::clone(&this);
        this.window.connect_destroy(move |_| {
            let _ = &owner;
        });
        companion.add_window(&this);
        this
    }

    pub fn present(&self) {
        self.window.present();
    }

    /// Open something the user asked for from outside the window, such as a
    /// link handed over by another app.
    pub fn open(self: &Rc<Self>, input: &str, new_tab: bool) {
        let tab = if new_tab {
            Some(self.add_tab(None, false))
        } else {
            self.selected_tab()
        };
        if let Some(tab) = tab {
            self.navigate(&tab, input);
        }
    }

    // --- What the companion asks of a window -------------------------------

    /// Active, and not minimised or hidden.
    pub fn in_front(&self) -> bool {
        let hidden = self
            .window
            .surface()
            .and_downcast::<gdk::Toplevel>()
            .is_some_and(|top| {
                top.state()
                    .intersects(gdk::ToplevelState::SUSPENDED | gdk::ToplevelState::MINIMIZED)
            });
        self.window.is_active() && !hidden
    }

    /// The tab on screen is playing sound.
    pub fn sound_on_screen(&self) -> bool {
        self.selected_tab().is_some_and(|tab| audible(&tab.view))
    }

    /// The addresses of the tabs other than the one on screen.
    pub fn other_tabs(&self, in_front: bool) -> Vec<String> {
        let selected = self.selected.get();
        self.tabs
            .borrow()
            .iter()
            .filter(|t| !(in_front && t.id == selected))
            .map(|t| t.view.uri().map(|u| u.to_string()).unwrap_or_default())
            .collect()
    }

    /// The address of the page on screen as the wisp sees it: nothing while
    /// a failure page stands in for it.
    pub fn attended_uri(&self) -> String {
        match self.selected_tab() {
            Some(tab) if tab.failed.borrow().is_none() => tab
                .view
                .uri()
                .map(|uri| uri.to_string())
                .unwrap_or_default(),
            _ => String::new(),
        }
    }

    pub fn selected_uri(&self) -> String {
        self.selected_tab()
            .and_then(|tab| tab.view.uri())
            .map(|uri| uri.to_string())
            .unwrap_or_default()
    }

    /// Used by the feel lab to show its clock.
    pub fn set_title(&self, title: &str) {
        self.window.set_title(Some(title));
    }

    /// Show the wisp's question about `site`, or none.
    pub fn ask(&self, site: Option<String>) {
        if self.sent_ask.borrow().as_ref() == Some(&site) {
            return;
        }
        self.sent_ask.replace(Some(site.clone()));
        self.send_to_chrome(&ToChrome::Ask { site });
    }

    /// Offer someone to talk to, or stop.
    pub fn care(&self, open: bool, samaritans: bool) {
        if self.sent_care.replace(Some((open, samaritans))) == Some((open, samaritans)) {
            return;
        }
        self.send_to_chrome(&ToChrome::Care { open, samaritans });
    }

    pub fn send_to_chrome(&self, message: &ToChrome) {
        if let ToChrome::Wisp {
            dose,
            mode,
            night,
            private,
            welcome,
            ..
        } = message
            && let Some(wisp) = self.wisp.borrow().as_ref()
        {
            let mode = match mode {
                WispMode::Away => Mode::Away,
                WispMode::Draining => Mode::Draining,
                WispMode::Resting => Mode::Resting,
                WispMode::Nourishing => Mode::Nourishing,
                WispMode::Holding => Mode::Holding,
            };
            wisp.update(*dose, mode, *private, *night, *welcome);
        }
        // The native wisp takes every change of dose; the chrome only shows
        // words, so it hears about the rest.
        let skip_same = |sent: &RefCell<String>, key: String| sent.replace(key.clone()) == key;
        match message {
            ToChrome::Wisp { .. } => {
                let mut value = serde_json::to_value(message).expect("chrome messages serialise");
                value["dose"] = serde_json::Value::Null;
                if skip_same(&self.sent_wisp, value.to_string()) {
                    return;
                }
            }
            ToChrome::Tabs { .. } => {
                let key = serde_json::to_string(message).expect("chrome messages serialise");
                if skip_same(&self.sent_tabs, key) {
                    return;
                }
            }
            _ => {}
        }
        let view = match message {
            ToChrome::Tabs { .. } | ToChrome::TabIcon { .. } => &self.sidebar,
            ToChrome::State { .. }
            | ToChrome::FocusAddress
            | ToChrome::Find { .. }
            | ToChrome::Found { .. }
            | ToChrome::Window { .. }
            | ToChrome::Caption { .. }
            | ToChrome::Ask { .. }
            | ToChrome::Care { .. }
            | ToChrome::Wisp { .. } => &self.toolbar,
        };
        if let Some(view) = view.borrow().as_ref() {
            chrome::send(view, message);
        }
    }

    // --- Tabs ------------------------------------------------------------------

    fn selected_tab(&self) -> Option<Rc<Tab>> {
        self.tab(self.selected.get())
    }

    fn tab(&self, id: u32) -> Option<Rc<Tab>> {
        self.tabs.borrow().iter().find(|t| t.id == id).cloned()
    }

    fn add_tab(self: &Rc<Self>, related: Option<&webkit::WebView>, select: bool) -> Rc<Tab> {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let view = tabs::new_view(related);
        let tab = Rc::new(Tab {
            id,
            view,
            fallback: RefCell::new(None),
            failed: RefCell::new(None),
            failure_pending: Cell::new(false),
            icon: RefCell::new(None),
        });
        self.stack.add_child(&tab.view);
        self.tabs.borrow_mut().push(Rc::clone(&tab));
        self.connect_tab(&tab);
        if select {
            self.select_tab(id);
        } else {
            self.push_tabs();
        }
        tab
    }

    fn select_tab(self: &Rc<Self>, id: u32) {
        let Some(tab) = self.tab(id) else { return };
        if let Some(previous) = self.selected_tab()
            && previous.id != id
        {
            self.close_find(false);
        }
        self.selected.set(id);
        self.stack.set_visible_child(&tab.view);
        let title = tab.view.title().filter(|t| !t.is_empty());
        self.window
            .set_title(Some(title.as_deref().unwrap_or("Glimmerwood")));
        self.push_tabs();
        self.push_state();
        let uri = tab.view.uri().map(|u| u.to_string()).unwrap_or_default();
        if uri.is_empty() || scheme::is_home(&uri) {
            self.focus_address();
        } else {
            tab.view.grab_focus();
        }
        // Home's greeting depends on the moment it's seen.
        self.push_page(&tab);
        self.companion.refresh();
    }

    /// Show Home in the tab on screen.
    fn go_home(&self) {
        if let Some(tab) = self.selected_tab() {
            tab.view.load_uri(HOME);
            self.focus_address();
        }
    }

    /// The wisp's history on Home: the tab on screen if it's
    /// Home, else the window's first Home tab, else a new one.
    fn show_wisp(self: &Rc<Self>) {
        let is_home = |tab: &Tab| tab.view.uri().is_some_and(|uri| scheme::is_home(&uri));
        let existing = self
            .selected_tab()
            .filter(|tab| is_home(tab))
            .or_else(|| self.tabs.borrow().iter().find(|tab| is_home(tab)).cloned());
        let tab = match existing {
            Some(tab) => {
                if tab.id != self.selected.get() {
                    self.select_tab(tab.id);
                }
                tab.view.evaluate_javascript(
                    "if (location.href.startsWith('glimmerwood://home/')) window.wispHome?.reveal()",
                    None,
                    None,
                    None::<&gio::Cancellable>,
                    |_| {},
                );
                tab
            }
            None => {
                let tab = self.add_tab(None, true);
                tab.view.load_uri(HOME_WISP);
                tab
            }
        };
        // Here to read, not to type an address.
        tab.view.grab_focus();
    }

    /// Settings: the window's Settings tab if it has one, else a new one.
    fn open_settings(self: &Rc<Self>) {
        let existing = self
            .tabs
            .borrow()
            .iter()
            .find(|tab| tab.view.uri().is_some_and(|uri| scheme::is_settings(&uri)))
            .cloned();
        let tab = match existing {
            Some(tab) => {
                self.select_tab(tab.id);
                tab
            }
            None => {
                self.companion.forget_lookup();
                let tab = self.add_tab(None, true);
                tab.view.load_uri(SETTINGS);
                tab
            }
        };
        tab.view.grab_focus();
    }

    /// A new tab showing Home.
    fn add_home_tab(self: &Rc<Self>) -> Rc<Tab> {
        let tab = self.add_tab(None, true);
        tab.view.load_uri(HOME);
        tab
    }

    /// Hand Home or Settings its data, if this tab is showing one. The script
    /// checks the page's own address first, so a page that has just replaced
    /// it never receives it. The data is also left on the page, because the
    /// page's script can still be loading its modules when WebKit says the
    /// load finished.
    fn push_page(&self, tab: &Tab) {
        let uri = tab.view.uri().map(|u| u.to_string()).unwrap_or_default();
        let (json, page, global) = if scheme::is_home(&uri) {
            let data = self.companion.home_data(attention::now());
            let json = serde_json::to_string(&data).expect("home data serialises");
            (json, "home", "wispHome")
        } else if scheme::is_settings(&uri) {
            let data = self.companion.settings_data();
            let json = serde_json::to_string(&data).expect("settings data serialises");
            (json, "settings", "wispSettings")
        } else {
            return;
        };
        tab.view.evaluate_javascript(
            &format!(
                "if (location.href.startsWith('glimmerwood://{page}/')) {{ window.{global}Data = {json}; window.{global}?.show(window.{global}Data); }}"
            ),
            None,
            None,
            None::<&gio::Cancellable>,
            move |result| {
                if let Err(err) = result {
                    eprintln!("glimmerwood: the {page} page didn't take its data: {err}");
                }
            },
        );
    }

    /// Refresh every tab showing Home or Settings, after something they show
    /// changed.
    pub fn refresh_pages(&self) {
        for tab in self.tabs.borrow().iter() {
            self.push_page(tab);
        }
    }

    fn toggle_bookmark(&self) {
        let Some(tab) = self.selected_tab() else {
            return;
        };
        let uri = tab.view.uri().map(|u| u.to_string()).unwrap_or_default();
        if !can_bookmark(&uri) || tab.failed.borrow().is_some() {
            return;
        }
        let title = tab
            .view
            .title()
            .filter(|t| !t.is_empty())
            .map_or_else(|| uri.clone(), |t| t.to_string());
        self.companion.toggle_bookmark(&uri, &title);
        self.push_state();
        self.companion.refresh_pages();
    }

    fn close_tab(self: &Rc<Self>, id: u32) {
        let Some(index) = self.tabs.borrow().iter().position(|t| t.id == id) else {
            return;
        };
        let tab = self.tabs.borrow_mut().remove(index);
        tab.view.stop_loading();
        self.stack.remove(&tab.view);
        let next = {
            let tabs = self.tabs.borrow();
            tabs.get(index).or_else(|| tabs.last()).map(|t| t.id)
        };
        match next {
            None => self.window.close(),
            Some(next) if self.selected.get() == id => self.select_tab(next),
            Some(_) => {
                self.push_tabs();
                self.companion.refresh();
            }
        }
    }

    fn step_tab(self: &Rc<Self>, by: isize) {
        let ids: Vec<u32> = self.tabs.borrow().iter().map(|t| t.id).collect();
        if let Some(pos) = ids.iter().position(|&id| id == self.selected.get()) {
            let next = (pos as isize + by).rem_euclid(ids.len() as isize) as usize;
            self.select_tab(ids[next]);
        }
    }

    fn select_nth(self: &Rc<Self>, n: usize) {
        let id = self.tabs.borrow().get(n).map(|t| t.id);
        if let Some(id) = id {
            self.select_tab(id);
        }
    }

    // --- Messages from the chrome ----------------------------------------------

    fn handle(self: &Rc<Self>, message: ToCore) {
        let tab = self.selected_tab();
        match message {
            ToCore::Ready {
                view: ChromeView::Sidebar,
            } => {
                self.sent_tabs.borrow_mut().clear();
                self.push_tabs();
                for tab in self.tabs.borrow().iter() {
                    self.push_icon(tab);
                }
            }
            ToCore::Ready {
                view: ChromeView::Toolbar,
            } => {
                self.push_state();
                self.push_window();
                self.sent_wisp.borrow_mut().clear();
                self.sent_ask.replace(None);
                self.sent_care.set(None);
                self.companion.chrome_ready();
                if self.companion.lab_shows_caption() {
                    self.send_to_chrome(&ToChrome::Caption { open: true });
                }
                let uri = self.selected_uri();
                if uri.is_empty() || scheme::is_home(&uri) {
                    self.focus_address();
                }
            }
            ToCore::Navigate { input } => {
                if let Some(tab) = tab {
                    self.navigate(&tab, &input);
                }
            }
            ToCore::Back => tab.iter().for_each(|t| t.view.go_back()),
            ToCore::Forward => tab.iter().for_each(|t| t.view.go_forward()),
            ToCore::Reload => tab.iter().for_each(|t| t.view.reload()),
            ToCore::Stop => tab.iter().for_each(|t| t.view.stop_loading()),
            ToCore::FocusPage => {
                if let Some(tab) = tab {
                    tab.view.grab_focus();
                }
            }
            ToCore::NewTab => {
                self.add_home_tab();
            }
            ToCore::ToggleBookmark => self.toggle_bookmark(),
            ToCore::GoHome => self.go_home(),
            ToCore::CloseTab { id } => self.close_tab(id),
            ToCore::ToggleMute { id } => {
                if let Some(tab) = self.tab(id) {
                    tab.view.set_is_muted(!tab.view.is_muted());
                }
            }
            ToCore::SelectTab { id } => self.select_tab(id),
            ToCore::ShowWisp => self.show_wisp(),
            ToCore::RateSite { site, rating } => self.companion.rate_site(&site, Some(rating)),
            ToCore::NotNow { site } => self.companion.not_now(&site),
            ToCore::CloseCare => self.companion.close_care(),
            ToCore::FindSupport { samaritans } => {
                let tab = self.add_tab(None, true);
                tab.view
                    .load_uri(if samaritans { SAMARITANS } else { HELPLINES });
            }
            ToCore::OpenSettings => self.open_settings(),
            ToCore::Find { query } => self.find(query),
            ToCore::FindNext { backwards } => self.find_next(backwards),
            ToCore::CloseFind => self.close_find(true),
            ToCore::ToolbarLayout {
                height,
                overlay_height,
                nook_right,
                nook_top,
                nook_width,
                nook_height,
            } => {
                self.paned.set_margin_top(height as i32);
                let cover = overlay_height.max(height) as i32;
                if self.toolbar_cover.replace(cover) != cover
                    && let Some(toolbar) = self.toolbar.borrow().as_ref()
                {
                    toolbar.queue_resize();
                }
                if let Some(wisp) = self.wisp.borrow().as_ref() {
                    let area = wisp.widget();
                    area.set_margin_end(nook_right as i32);
                    area.set_margin_top(nook_top as i32);
                    area.set_content_width(nook_width.max(1) as i32);
                    area.set_content_height(nook_height.max(1) as i32);
                }
            }
            ToCore::Minimize => self.window.minimize(),
            ToCore::ToggleMaximize => {
                if self.window.is_maximized() {
                    self.window.unmaximize();
                } else {
                    self.window.maximize();
                }
            }
            ToCore::CloseWindow => self.window.close(),
            ToCore::BeginMove => {
                let press = self.last_press.borrow().clone();
                // Only while that button is still held: a late message must
                // not start a move the user has already let go of.
                if let (Some((device, button, x, y, time)), Some(surface)) =
                    (press, self.window.surface())
                    && surface
                        .device_position(&device)
                        .is_some_and(|(_, _, mask)| mask.contains(gdk::ModifierType::BUTTON1_MASK))
                    && let Ok(top) = surface.downcast::<gdk::Toplevel>()
                {
                    top.begin_move(&device, button as i32, x, y, time);
                }
            }
        }
    }

    // --- Find in page ---------------------------------------------------------

    fn open_find(&self) {
        if self.selected_tab().is_none() {
            return;
        }
        self.finding.set(true);
        if let Some(toolbar) = self.toolbar.borrow().as_ref() {
            toolbar.grab_focus();
        }
        self.send_to_chrome(&ToChrome::Find { open: true });
    }

    /// Search the page on screen from the top, ignoring case and wrapping
    /// round at the end.
    fn find(&self, query: String) {
        let Some(finder) = self.selected_tab().and_then(|t| t.view.find_controller()) else {
            return;
        };
        self.find_query.replace(query.clone());
        if query.is_empty() {
            self.find_pending.set(0);
            finder.search_finish();
            self.send_to_chrome(&ToChrome::Found {
                query,
                summary: String::new(),
            });
            return;
        }
        let options = webkit::FindOptions::CASE_INSENSITIVE | webkit::FindOptions::WRAP_AROUND;
        self.find_pending.set(self.find_pending.get() + 1);
        finder.search(&query, options.bits(), find::MAX_MATCHES);
    }

    /// Ctrl+G and Enter in the find bar. With the bar closed, it opens.
    fn find_next(&self, backwards: bool) {
        let Some(finder) = self.selected_tab().and_then(|t| t.view.find_controller()) else {
            return;
        };
        if !self.finding.get() || finder.search_text().is_none_or(|t| t.is_empty()) {
            self.open_find();
            return;
        }
        if backwards {
            finder.search_previous();
        } else {
            finder.search_next();
        }
    }

    /// Clear the highlights and close the bar. `to_page`: the user closed
    /// it, so the caret goes back to the page.
    fn close_find(&self, to_page: bool) {
        if !self.finding.replace(false) {
            return;
        }
        self.find_pending.set(0);
        if let Some(tab) = self.selected_tab() {
            if let Some(finder) = tab.view.find_controller() {
                finder.search_finish();
            }
            if to_page {
                tab.view.grab_focus();
            }
        }
        self.send_to_chrome(&ToChrome::Find { open: false });
    }

    fn navigate(&self, tab: &Tab, input: &str) {
        let Some(target) = nav::resolve(input) else {
            return;
        };
        tab.view.load_uri(&target.uri);
        tab.fallback
            .replace(target.fallback.is_some().then_some(target));
        tab.view.grab_focus();
    }

    fn focus_address(&self) {
        if let Some(toolbar) = self.toolbar.borrow().as_ref() {
            toolbar.grab_focus();
            chrome::send(toolbar, &ToChrome::FocusAddress);
        }
    }

    /// Window buttons only while the window floats: not when it is maximised,
    /// full screen, or tiled on every side. Tiled against one or two edges
    /// (half the screen on a floating desktop) it still needs its buttons.
    fn push_window(&self) {
        use gdk::ToplevelState as S;
        let edges = S::TOP_TILED | S::BOTTOM_TILED | S::LEFT_TILED | S::RIGHT_TILED;
        let floating = self
            .window
            .surface()
            .and_downcast::<gdk::Toplevel>()
            .is_none_or(|top| {
                let state = top.state();
                let tiled =
                    state.contains(edges) || (state.contains(S::TILED) && !state.intersects(edges));
                !(tiled || state.intersects(S::MAXIMIZED | S::FULLSCREEN))
            });
        self.send_to_chrome(&ToChrome::Window { floating });
    }

    // --- Tab signals -----------------------------------------------------------

    fn connect_tab(self: &Rc<Self>, tab: &Rc<Tab>) {
        let view = &tab.view;
        let id = tab.id;

        let weak = Rc::downgrade(self);
        let changed = move |uri_changed: bool| {
            let Some(this) = weak.upgrade() else { return };
            this.push_tabs();
            if this.selected.get() == id {
                this.queue_state();
                if uri_changed {
                    this.companion.refresh();
                }
            }
        };
        view.connect_uri_notify(glib::clone!(
            #[strong]
            changed,
            move |_| changed(true)
        ));
        view.connect_is_loading_notify(glib::clone!(
            #[strong]
            changed,
            move |_| changed(false)
        ));
        view.connect_estimated_load_progress_notify(glib::clone!(
            #[strong]
            changed,
            move |_| changed(false)
        ));

        let weak = Rc::downgrade(self);
        view.connect_title_notify(move |view| {
            let Some(this) = weak.upgrade() else { return };
            changed(false);
            if this.selected.get() == id {
                let title = view.title().filter(|t| !t.is_empty());
                this.window
                    .set_title(Some(title.as_deref().unwrap_or("Glimmerwood")));
            }
        });

        let weak = Rc::downgrade(self);
        let weak_tab = Rc::downgrade(tab);
        let icon_changed = move |view: &webkit::WebView| {
            if let (Some(this), Some(tab)) = (weak.upgrade(), weak_tab.upgrade()) {
                // Home and Settings have no favicon; they wear the wisp.
                let icon = if view.uri().is_some_and(|uri| scheme::is_local_page(&uri)) {
                    Some(home_icon())
                } else {
                    view.favicon().map(|icon| data_url(&icon))
                };
                if *tab.icon.borrow() != icon {
                    tab.icon.replace(icon);
                    this.push_icon(&tab);
                }
            }
        };
        view.connect_favicon_notify(glib::clone!(
            #[strong]
            icon_changed,
            move |view| icon_changed(view)
        ));
        view.connect_uri_notify(move |view| icon_changed(view));

        if let Some(finder) = view.find_controller() {
            let weak = Rc::downgrade(self);
            let found = move |matches: Option<u32>| {
                if let Some(this) = weak.upgrade()
                    && this.selected.get() == id
                    && this.finding.get()
                    && this.find_pending.get() > 0
                    && this.find_pending.replace(this.find_pending.get() - 1) == 1
                {
                    this.send_to_chrome(&ToChrome::Found {
                        query: this.find_query.borrow().clone(),
                        summary: find::summary(matches),
                    });
                }
            };
            finder.connect_found_text(glib::clone!(
                #[strong]
                found,
                move |_, matches| found(Some(matches))
            ));
            finder.connect_failed_to_find_text(move |_| found(None));
        }

        // Sound on screen keeps the user present; the speaker mark on the
        // tab follows it either way.
        let weak = Rc::downgrade(self);
        let sound_changed = move |_view: &webkit::WebView| {
            let Some(this) = weak.upgrade() else {
                return;
            };
            this.push_tabs();
            this.companion.refresh();
        };
        view.connect_is_playing_audio_notify(glib::clone!(
            #[strong]
            sound_changed,
            move |view| sound_changed(view)
        ));
        view.connect_is_muted_notify(move |view| sound_changed(view));

        let weak_tab = Rc::downgrade(tab);
        let weak_self = Rc::downgrade(self);
        view.connect_load_changed(move |_, event| {
            let Some(tab) = weak_tab.upgrade() else {
                return;
            };
            match event {
                webkit::LoadEvent::Committed => {
                    tab.fallback.take();
                }
                webkit::LoadEvent::Finished
                    if tab
                        .view
                        .uri()
                        .is_some_and(|uri| scheme::is_local_page(&uri)) =>
                {
                    if let Some(this) = weak_self.upgrade() {
                        this.push_page(&tab);
                    }
                }
                // The failure page's own load starts once; any other load is
                // a new navigation, and the failure is over.
                webkit::LoadEvent::Started if !tab.failure_pending.replace(false) => {
                    tab.failed.take();
                    if let Some(this) = weak_self.upgrade()
                        && this.selected.get() == tab.id
                    {
                        this.close_find(false);
                    }
                }
                _ => {}
            }
        });

        // A page's process crashed or was killed for memory: say so rather
        // than leave a blank tab.
        let weak_tab = Rc::downgrade(tab);
        view.connect_web_process_terminated(move |view, reason| {
            let Some(tab) = weak_tab.upgrade() else {
                return;
            };
            let uri = view.uri().map(|u| u.to_string()).unwrap_or_default();
            let why = match reason {
                webkit::WebProcessTerminationReason::ExceededMemoryLimit => {
                    "The page used more memory than it was allowed."
                }
                _ => "The page stopped working.",
            };
            tab.failed.replace(Some(uri.clone()));
            tab.failure_pending.set(true);
            view.load_alternate_html(
                &failure::page(&uri, &Reason::Other(why.to_owned())),
                &uri,
                None,
            );
        });

        // A certificate problem is never a reason to retry without encryption.
        let weak_tab = Rc::downgrade(tab);
        view.connect_load_failed_with_tls_errors(move |_, _, _, _| {
            if let Some(tab) = weak_tab.upgrade() {
                tab.fallback.take();
            }
            false
        });

        let weak_tab = Rc::downgrade(tab);
        view.connect_load_failed(move |view, _, failing_uri, error| {
            let Some(tab) = weak_tab.upgrade() else {
                return false;
            };
            // A fallback belongs to one attempt. The failure of a page this
            // navigation replaced may arrive after it, and must leave it be.
            let fallback = {
                let mut slot = tab.fallback.borrow_mut();
                match slot.as_ref() {
                    Some(target) if nav::same_address(&target.uri, failing_uri) => {
                        slot.take().and_then(|target| target.fallback)
                    }
                    _ => None,
                }
            };
            // The user stopped it, or a newer navigation replaced it.
            if error.matches(webkit::NetworkError::Cancelled)
                || error.matches(webkit::PolicyError::FrameLoadInterruptedByPolicyChange)
            {
                return false;
            }
            match fallback {
                Some(http) => view.load_uri(&http),
                None => {
                    tab.failed.replace(Some(failing_uri.to_owned()));
                    tab.failure_pending.set(true);
                    view.load_alternate_html(
                        &failure::page(failing_uri, &Reason::of(error)),
                        failing_uri,
                        None,
                    );
                }
            }
            true
        });

        // Links that ask for a new window open as tabs: in the background
        // for a middle- or Ctrl-click, in front otherwise (through `create`).
        let weak = Rc::downgrade(self);
        view.connect_decide_policy(move |view, decision, kind| {
            // The buttons on Home and Settings are links to
            // glimmerwood://home/do/... and glimmerwood://settings/do/...,
            // caught here and only honoured while the tab is showing that page.
            if kind == webkit::PolicyDecisionType::NavigationAction {
                let target = decision
                    .downcast_ref::<webkit::NavigationPolicyDecision>()
                    .and_then(|d| d.navigation_action())
                    .and_then(|a| a.request())
                    .and_then(|r| r.uri());
                let Some(target) = target.as_deref() else {
                    return false;
                };
                let showing = view.uri().map(|u| u.to_string()).unwrap_or_default();
                if let Some(action) = target.strip_prefix(HOME_ACTIONS) {
                    decision.ignore();
                    if let Some(this) = weak.upgrade()
                        && scheme::is_home(&showing)
                        && this.companion.home_action(action)
                    {
                        this.companion.refresh_pages();
                    }
                    return true;
                }
                if let Some(action) = target.strip_prefix(SETTINGS_ACTIONS) {
                    decision.ignore();
                    if let Some(this) = weak.upgrade()
                        && scheme::is_settings(&showing)
                    {
                        this.companion.settings_action(action);
                    }
                    return true;
                }
                return false;
            }
            if kind != webkit::PolicyDecisionType::NewWindowAction {
                return false;
            }
            let Some(action) = decision
                .downcast_ref::<webkit::NavigationPolicyDecision>()
                .and_then(|d| d.navigation_action())
            else {
                return false;
            };
            let background = action.mouse_button() == 2
                || gdk::ModifierType::from_bits_truncate(action.modifiers())
                    .contains(gdk::ModifierType::CONTROL_MASK);
            if !background {
                decision.use_();
                return true;
            }
            decision.ignore();
            if let (Some(this), Some(uri)) =
                (weak.upgrade(), action.request().and_then(|r| r.uri()))
            {
                let tab = this.add_tab(None, false);
                tab.view.load_uri(&uri);
            }
            true
        });

        let weak = Rc::downgrade(self);
        view.connect_create(move |view, _| {
            let this = weak.upgrade()?;
            let tab = this.add_tab(Some(view), true);
            Some(tab.view.clone().upcast())
        });

        let weak = Rc::downgrade(self);
        view.connect_close(move |_| {
            if let Some(this) = weak.upgrade() {
                this.close_tab(id);
            }
        });
    }

    // --- Attention -------------------------------------------------------------

    fn watch_attention(self: &Rc<Self>) {
        let companion = Rc::downgrade(&self.companion);
        let input = move || {
            if let Some(companion) = companion.upgrade() {
                companion.input();
            }
        };

        // Capture phase: seen before the page gets the event, and never
        // claimed or stopped. Only the fact of input is used, never its content.
        let key = gtk::EventControllerKey::new();
        key.set_propagation_phase(gtk::PropagationPhase::Capture);
        let typed = Rc::downgrade(&self.companion);
        key.connect_key_pressed(glib::clone!(
            #[strong]
            input,
            move |_, _, _, _| {
                input();
                if let Some(companion) = typed.upgrade() {
                    companion.typed();
                }
                glib::Propagation::Proceed
            }
        ));
        self.window.add_controller(key);

        let motion = gtk::EventControllerMotion::new();
        motion.set_propagation_phase(gtk::PropagationPhase::Capture);
        motion.connect_motion(glib::clone!(
            #[strong]
            input,
            move |_, _, _| input()
        ));
        self.window.add_controller(motion);

        let click = gtk::GestureClick::new();
        click.set_button(0);
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        click.connect_pressed(glib::clone!(
            #[strong]
            input,
            move |gesture, _, x, y| {
                input();
                // Kept in case the chrome asks to drag the window from here.
                if let (Some(this), Some(device)) = (weak.upgrade(), gesture.current_event_device())
                {
                    this.last_press.replace(Some((
                        device,
                        gesture.current_button(),
                        x,
                        y,
                        gesture.current_event_time(),
                    )));
                }
            }
        ));
        self.window.add_controller(click);

        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
        scroll.connect_scroll(move |_, _, _| {
            input();
            glib::Propagation::Proceed
        });
        self.window.add_controller(scroll);

        let weak = Rc::downgrade(self);
        let refresh = move || {
            if let Some(this) = weak.upgrade() {
                this.companion.refresh();
            }
        };
        self.window.connect_is_active_notify(glib::clone!(
            #[strong]
            refresh,
            move |_| refresh()
        ));
        let weak = Rc::downgrade(self);
        self.window.connect_realize(move |window| {
            if let Some(top) = window.surface().and_downcast::<gdk::Toplevel>() {
                let weak = weak.clone();
                top.connect_state_notify(glib::clone!(
                    #[strong]
                    refresh,
                    move |_| {
                        refresh();
                        if let Some(this) = weak.upgrade() {
                            this.push_window();
                        }
                    }
                ));
            }
        });
    }

    // --- Actions ---------------------------------------------------------------

    fn install_actions(self: &Rc<Self>) {
        let add = |name: &str, run: Action| {
            let action = gio::SimpleAction::new(name, None);
            let weak: Weak<Window> = Rc::downgrade(self);
            action.connect_activate(move |_, _| {
                if let Some(this) = weak.upgrade() {
                    run(&this);
                }
            });
            self.window.add_action(&action);
        };
        add("focus-address", Box::new(|w| w.focus_address()));
        add(
            "back",
            Box::new(|w| w.selected_tab().iter().for_each(|t| t.view.go_back())),
        );
        add(
            "forward",
            Box::new(|w| w.selected_tab().iter().for_each(|t| t.view.go_forward())),
        );
        add(
            "reload",
            Box::new(|w| w.selected_tab().iter().for_each(|t| t.view.reload())),
        );
        add(
            "new-tab",
            Box::new(|w| {
                w.add_home_tab();
            }),
        );
        add("close-tab", Box::new(|w| w.close_tab(w.selected.get())));
        add("bookmark", Box::new(|w| w.toggle_bookmark()));
        add("home", Box::new(|w| w.go_home()));
        add("find", Box::new(|w| w.open_find()));
        add("settings", Box::new(|w| w.open_settings()));
        add("find-next", Box::new(|w| w.find_next(false)));
        add("find-previous", Box::new(|w| w.find_next(true)));
        add("next-tab", Box::new(|w| w.step_tab(1)));
        add("previous-tab", Box::new(|w| w.step_tab(-1)));
        for n in 1..=NUMBERED_TABS {
            add(
                &format!("tab-{n}"),
                Box::new(move |w| w.select_nth(n as usize - 1)),
            );
        }
        add(
            "last-tab",
            Box::new(|w| {
                let last = w.tabs.borrow().len().saturating_sub(1);
                w.select_nth(last);
            }),
        );
    }

    // --- State for the chrome --------------------------------------------------

    fn push_tabs(&self) {
        let tabs = self
            .tabs
            .borrow()
            .iter()
            .map(|t| TabInfo {
                id: t.id,
                title: t
                    .view
                    .title()
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .or_else(|| t.view.uri().map(|u| u.to_string()))
                    .unwrap_or_default(),
                host: t
                    .view
                    .uri()
                    .and_then(|uri| glib::Uri::parse(&uri, glib::UriFlags::NONE).ok())
                    .and_then(|uri| uri.host())
                    .map(|host| host.to_string())
                    .unwrap_or_default(),
                loading: t.view.is_loading(),
                sound: match (t.view.is_playing_audio(), t.view.is_muted()) {
                    (_, true) => TabSound::Muted,
                    (true, false) => TabSound::Playing,
                    (false, false) => TabSound::Silent,
                },
            })
            .collect();
        self.send_to_chrome(&ToChrome::Tabs {
            tabs,
            selected: self.selected.get(),
        });
    }

    fn push_icon(&self, tab: &Tab) {
        self.send_to_chrome(&ToChrome::TabIcon {
            id: tab.id,
            icon: tab.icon.borrow().clone(),
        });
    }

    /// Keep the tab column between its limits, and remember where the user
    /// left it once they stop dragging.
    fn remember_tab_column_width(self: &Rc<Self>) {
        let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
        self.paned.connect_position_notify(move |paned| {
            let width = paned.position();
            let kept = width.clamp(prefs::MIN_TAB_COLUMN_WIDTH, prefs::MAX_TAB_COLUMN_WIDTH);
            if kept != width {
                paned.set_position(kept);
                return;
            }
            // A window squeezed narrower pushes the handle to its limit;
            // that isn't the user choosing a width.
            if width >= paned.max_position() {
                return;
            }
            if let Some(source) = pending.take() {
                source.remove();
            }
            let pending_inner = Rc::clone(&pending);
            let source = glib::timeout_add_local_once(
                std::time::Duration::from_millis(SAVE_WIDTH_AFTER_MS),
                move || {
                    pending_inner.take();
                    if kept != prefs::tab_column_width() {
                        prefs::set_tab_column_width(kept);
                    }
                },
            );
            pending.replace(Some(source));
        });
    }

    /// WebKit fires several notifications per load step; send the chrome one
    /// state per main-loop turn.
    fn queue_state(self: &Rc<Self>) {
        if self.state_queued.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(this) = weak.upgrade() {
                this.state_queued.set(false);
                this.push_state();
            }
        });
    }

    fn push_state(&self) {
        let Some(tab) = self.selected_tab() else {
            return;
        };
        let view = &tab.view;
        let uri = view.uri().map(|u| u.to_string()).unwrap_or_default();
        let can_bookmark = can_bookmark(&uri) && tab.failed.borrow().is_none();
        let state = ToChrome::State {
            security: nav::security(&uri),
            can_bookmark,
            bookmarked: can_bookmark && self.companion.is_bookmarked(&uri),
            // Home and a blank tab leave the field empty, ready to type in.
            uri: if uri == "about:blank" || scheme::is_local_page(&uri) {
                String::new()
            } else {
                uri
            },
            title: view.title().map(|t| t.to_string()).unwrap_or_default(),
            loading: view.is_loading(),
            progress: view.estimated_load_progress(),
            can_go_back: view.can_go_back(),
            can_go_forward: view.can_go_forward(),
        };
        self.send_to_chrome(&state);
    }
}

/// Where new tabs start, and where its buttons point.
const HOME: &str = "glimmerwood://home/";
const HOME_ACTIONS: &str = "glimmerwood://home/do/";
const HOME_WISP: &str = "glimmerwood://home/#wisp";
const SETTINGS: &str = "glimmerwood://settings/";
/// Where the care note leads: an international directory of free, confidential
/// helplines, and Samaritans in the UK and Ireland.
const HELPLINES: &str = "https://findahelpline.com/";
const SAMARITANS: &str = "https://www.samaritans.org/how-we-can-help/contact-samaritan/";
const SETTINGS_ACTIONS: &str = "glimmerwood://settings/do/";

/// Only web pages can be bookmarked.
fn can_bookmark(uri: &str) -> bool {
    matches!(nav::security(uri), Security::Secure | Security::NotSecure)
}

/// The app's own icon, for tabs showing Home.
fn home_icon() -> String {
    thread_local! {
        static ICON: String = {
            let svg = gio::resources_lookup_data(
                "/io/github/peterwalker78/Glimmerwood/home/wisp.svg",
                gio::ResourceLookupFlags::NONE,
            )
            .expect("the icon is bundled");
            format!("data:image/svg+xml;base64,{}", glib::base64_encode(&svg))
        };
    }
    ICON.with(Clone::clone)
}

/// A site icon as a `data:` URL the chrome can show.
fn data_url(icon: &gdk::Texture) -> String {
    let png = icon.save_to_png_bytes();
    format!("data:image/png;base64,{}", glib::base64_encode(&png))
}

/// Playing sound the user can hear: not muted.
fn audible(view: &webkit::WebView) -> bool {
    view.is_playing_audio() && !view.is_muted()
}
