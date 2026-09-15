//! Messages between the chrome webview and the core.
//!
//! Every type here is declared once, through the macros below, which also
//! write the matching TypeScript. `ui/protocol.gen.ts` is checked against it by
//! a test; after changing a message, run `scripts/protocol` to rewrite the
//! file.

use serde::{Deserialize, Serialize};

/// The TypeScript spelling of a Rust type used on the wire.
#[cfg(test)]
trait Ts {
    fn ts() -> String;
}

#[cfg(test)]
mod ts_primitives {
    use super::Ts;

    impl Ts for String {
        fn ts() -> String {
            "string".into()
        }
    }
    impl Ts for bool {
        fn ts() -> String {
            "boolean".into()
        }
    }
    impl Ts for f64 {
        fn ts() -> String {
            "number".into()
        }
    }
    impl Ts for i64 {
        fn ts() -> String {
            "number".into()
        }
    }
    impl Ts for u32 {
        fn ts() -> String {
            "number".into()
        }
    }
    impl<T: Ts> Ts for Vec<T> {
        fn ts() -> String {
            let item = T::ts();
            if item.contains(' ') {
                format!("({item})[]")
            } else {
                format!("{item}[]")
            }
        }
    }
    impl<T: Ts> Ts for Option<T> {
        fn ts() -> String {
            format!("{} | null", T::ts())
        }
    }
}

#[cfg(test)]
fn fields_ts(fields: &[(&str, String)]) -> String {
    fields
        .iter()
        .map(|(name, ty)| format!("{name}: {ty}"))
        .collect::<Vec<_>>()
        .join("; ")
}

/// A plain object: `{ field: value, ... }` on the wire.
macro_rules! record {
    (
        $(#[$meta:meta])*
        pub struct $name:ident { $($(#[doc = $doc:literal])* pub $field:ident: $ty:ty),* $(,)? }
    ) => {
        $(#[$meta])*
        pub struct $name { $($(#[doc = $doc])* pub $field: $ty),* }

        #[cfg(test)]
        impl Ts for $name {
            fn ts() -> String {
                stringify!($name).into()
            }
        }

        #[cfg(test)]
        impl $name {
            fn declaration() -> String {
                let fields = [$((stringify!($field), <$ty as Ts>::ts())),*];
                format!("export type {} = {{ {} }};\n", stringify!($name), fields_ts(&fields))
            }
        }
    };
}

/// A tagged union: `{ "type": "snake_case_variant", ...fields }` on the wire.
macro_rules! messages {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $(
                $(#[doc = $doc:literal])*
                $variant:ident $({ $($field:ident: $ty:ty),* $(,)? })?
            ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[serde(tag = "type", rename_all = "snake_case")]
        pub enum $name {
            $(
                $(#[doc = $doc])*
                $variant $({ $($field: $ty),* })?
            ),*
        }

        #[cfg(test)]
        impl $name {
            fn declaration() -> String {
                let variants: Vec<String> = vec![$(
                    {
                        #[allow(unused_mut)]
                        let mut fields = vec![("type", format!("\"{}\"", snake_case(stringify!($variant))))];
                        $($(fields.push((stringify!($field), <$ty as Ts>::ts()));)*)?
                        format!("{{ {} }}", fields_ts(&fields))
                    }
                ),*];
                format!("export type {} =\n  | {};\n", stringify!($name), variants.join("\n  | "))
            }
        }
    };
}

/// A plain string union: `"snake_case_variant"` on the wire.
macro_rules! string_union {
    (
        $(#[$meta:meta])*
        pub enum $name:ident { $($(#[doc = $doc:literal])* $variant:ident),* $(,)? }
    ) => {
        $(#[$meta])*
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($(#[doc = $doc])* $variant),* }

        #[cfg(test)]
        impl Ts for $name {
            fn ts() -> String {
                stringify!($name).into()
            }
        }

        #[cfg(test)]
        impl $name {
            fn declaration() -> String {
                let variants = [$(format!("\"{}\"", snake_case(stringify!($variant)))),*];
                format!("export type {} = {};\n", stringify!($name), variants.join(" | "))
            }
        }
    };
}

string_union! {
    /// How private the connection to the page in the tab is.
    #[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Security {
        /// Loaded over HTTPS.
        Secure,
        /// Loaded over plain HTTP, readable by anyone on the network path.
        NotSecure,
        /// Nothing from the network: a blank tab or a local page.
        Local,
    }
}

string_union! {
    /// The two chrome pages: the toolbar across the top (with the wisp's
    /// nook) and the column of tabs down the left.
    #[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
    pub enum ChromeView { Sidebar, Toolbar }
}

string_union! {
    /// Whether a tab is making sound, for its mark in the tab column.
    #[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
    pub enum TabSound { Silent, Playing, Muted }
}

string_union! {
    #[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
    pub enum WispPhase { Rested, Engaged, Clouded, Drained }
}

string_union! {
    /// What the dose is doing, which colours how the wisp moves.
    #[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
    pub enum WispMode { Away, Draining, Resting, Nourishing, Holding }
}

string_union! {
    #[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
    pub enum WispTrend { Rising, Falling, Steady }
}

string_union! {
    #[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
    pub enum CaptionKind {
        /// A draining site; `label` is its list entry.
        Wearing,
        /// A nourishing site; `label` is its list entry.
        Restoring,
        /// Ordinary sites; `label` is the site when describing now, empty
        /// when summing up the last 15 minutes.
        OrdinarySites,
        /// A site on no list, which holds the dose steady; `label` is the
        /// site. Only ever describes now.
        Holding,
        /// A tab playing sound that changes nothing; `label` is the site.
        /// Only ever describes now.
        Playing,
        /// A private site: nothing about it is named.
        Private,
        Away,
    }
}

record! {
    /// One line of the hover caption.
    #[derive(Serialize, Clone, Debug, PartialEq)]
    pub struct CaptionLine {
        pub kind: CaptionKind,
        pub label: String,
        /// 1 to 3 by share of the movement; 0 on lines about now.
        pub bars: u32,
        /// About a tab that's playing sound rather than the page on screen.
        pub heard: bool,
    }
}

record! {
    #[derive(Serialize, Clone, Debug, PartialEq)]
    pub struct TabInfo {
        pub id: u32,
        pub title: String,
        /// The site, for the letter shown when it has no icon.
        pub host: String,
        pub loading: bool,
        pub sound: TabSound,
    }
}

string_union! {
    #[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
    pub enum DayPart { Morning, Day, Evening, Night }
}

string_union! {
    #[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
    pub enum PlantKind { Moss, Fern, Flower }
}

record! {
    /// Something Home explains until the user says "Got it".
    #[derive(Serialize, Clone, Debug, PartialEq)]
    pub struct HomeExplain {
        pub topic: String,
        pub text: String,
    }
}

record! {
    #[derive(Serialize, Clone, Debug, PartialEq)]
    pub struct HomePlace {
        pub name: String,
        pub line: String,
        pub url: String,
        /// One the user already returns to.
        pub yours: bool,
    }
}

record! {
    #[derive(Serialize, Clone, Debug, PartialEq)]
    pub struct HomeBookmark {
        pub id: i64,
        pub title: String,
        pub url: String,
        pub host: String,
    }
}

record! {
    /// A day that grew the garden, oldest first.
    #[derive(Serialize, Clone, Debug, PartialEq)]
    pub struct HomePlant {
        pub kind: PlantKind,
        pub fireflies: bool,
    }
}

string_union! {
    /// Kinds of time in the wisp's history: on nourishing sites, on ordinary
    /// or unlisted ones, and on draining ones. Time away isn't counted.
    #[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
    pub enum TimeKind { Restoring, Everyday, Wearing }
}

record! {
    /// A point on today's line.
    #[derive(Serialize, Clone, Debug, PartialEq)]
    pub struct DosePoint {
        /// Minutes since the day began (05:00).
        pub minute: u32,
        pub dose: f64,
        /// Nothing was recorded for a while before this point: the browser
        /// was closed.
        pub gap: bool,
    }
}

record! {
    #[derive(Serialize, Clone, Debug, PartialEq)]
    pub struct TimeShare {
        pub kind: TimeKind,
        /// Its part of the time counted, from 0 to 1; only for drawing.
        pub share: f64,
    }
}

record! {
    /// One of the last seven days.
    #[derive(Serialize, Clone, Debug, PartialEq)]
    pub struct DiaryDay {
        pub label: String,
        /// On or after the first day anything was recorded.
        pub known: bool,
        /// Time here compared with the busiest of the seven, from 0 to 1.
        pub size: f64,
        pub shares: Vec<TimeShare>,
        /// How heavy the wisp got at its heaviest; none on a day away.
        pub peak: Option<WispPhase>,
        pub plant: Option<PlantKind>,
    }
}

record! {
    /// One of the weeks before the last seven days.
    #[derive(Serialize, Clone, Debug, PartialEq)]
    pub struct DiaryWeek {
        pub label: String,
        /// Time here compared with the busiest week shown, from 0 to 1.
        pub size: f64,
        pub shares: Vec<TimeShare>,
        /// What its days grew, oldest first.
        pub plants: Vec<PlantKind>,
    }
}

record! {
    /// A list entry that moved the wisp this week.
    #[derive(Serialize, Clone, Debug, PartialEq)]
    pub struct DiarySite {
        pub label: String,
        pub kind: TimeKind,
        /// 1 to 3, compared with the site with the most time.
        pub bars: u32,
    }
}

record! {
    /// The wisp's history, where clicking the wisp leads.
    #[derive(Serialize, Clone, Debug, PartialEq)]
    pub struct HomeWisp {
        pub today: Vec<DosePoint>,
        pub now_minute: u32,
        /// When the day begins, in minutes after midnight, for the clock
        /// under the line.
        pub day_start_minute: u32,
        /// The night window, in minutes since the day began.
        pub night_from: u32,
        pub night_until: u32,
        /// Where Busy, Clouded and Sleepy begin.
        pub engaged: f64,
        pub clouded: f64,
        pub drained: f64,
        /// The last seven days, oldest first, ending today.
        pub week: Vec<DiaryDay>,
        /// Up to four weeks before those, newest first.
        pub earlier: Vec<DiaryWeek>,
        pub sites: Vec<DiarySite>,
    }
}

record! {
    /// Everything Home shows, handed to the page by the core. The
    /// page never asks for anything: its buttons are links the core catches.
    #[derive(Serialize, Clone, Debug, PartialEq)]
    pub struct HomeData {
        pub title: String,
        pub line: String,
        pub explain: Option<HomeExplain>,
        pub part: DayPart,
        pub places: Vec<HomePlace>,
        pub bookmarks: Vec<HomeBookmark>,
        pub plants: Vec<HomePlant>,
        /// Where the garden puts things; fixed for this install.
        pub seed: u32,
        pub wisp: HomeWisp,
    }
}

messages! {
    /// Sent by the chrome.
    #[derive(Deserialize, Debug, PartialEq)]
    pub enum ToCore {
        /// A chrome page has loaded and can receive messages.
        Ready { view: ChromeView },
        /// The user submitted the address field.
        Navigate { input: String },
        Back,
        Forward,
        Reload,
        Stop,
        /// The user left the address field with Escape.
        FocusPage,
        NewTab,
        CloseTab { id: u32 },
        /// Show Home in the tab on screen.
        GoHome,
        /// Star or unstar the page on screen.
        ToggleBookmark,
        /// Mute a tab playing sound, or unmute it.
        ToggleMute { id: u32 },
        /// Find `query` in the page on screen, from the top; empty clears it.
        Find { query: String },
        /// Move to the next match, or the previous one.
        FindNext { backwards: bool },
        /// The find bar closed: clear the highlights and go back to the page.
        CloseFind,
        SelectTab { id: u32 },
        /// The wisp was clicked, or pressed from the keyboard: show its
        /// history on Home.
        ShowWisp,
        /// The toolbar's size: `height` is the bar pages sit below;
        /// `overlay_height` is the full height it needs, including the hover
        /// caption when open, which floats over the page. The nook is where
        /// the native wisp is drawn, measured from the top right.
        ToolbarLayout {
            height: u32,
            overlay_height: u32,
            nook_right: u32,
            nook_top: u32,
            nook_width: u32,
            nook_height: u32,
        },
        /// Window buttons, shown while the window floats.
        Minimize,
        ToggleMaximize,
        CloseWindow,
        /// A press on empty chrome: move the window with the pointer.
        BeginMove,
    }
}

messages! {
    /// Sent to the chrome.
    #[derive(Serialize, Debug, PartialEq)]
    pub enum ToChrome {
        /// Everything the toolbar shows about the current tab.
        State {
            uri: String,
            title: String,
            loading: bool,
            progress: f64,
            can_go_back: bool,
            can_go_forward: bool,
            security: Security,
            can_bookmark: bool,
            bookmarked: bool,
        },
        Tabs { tabs: Vec<TabInfo>, selected: u32 },
        /// A tab's site icon as a `data:` URL, or none. Sent only when it
        /// changes, not with every tab update.
        TabIcon { id: u32, icon: Option<String> },
        /// Put the caret in the address field and select its contents.
        FocusAddress,
        /// Open the find bar with the caret in it, or close it (another tab
        /// was selected, or the page moved on).
        Find { open: bool },
        /// How a search went, in words. `query` says which search it answers,
        /// so a late answer to an older one is ignored.
        Found { query: String, summary: String },
        /// Whether the window floats (window buttons shown) or is tiled,
        /// maximised or full screen (no buttons).
        Window { floating: bool },
        /// Open or close the hover caption: the pointer entered or left the
        /// wisp, which is drawn natively above the chrome.
        Caption { open: bool },
        /// The companion's state, sent when it changes and while the dose
        /// moves. The native wisp takes the dose, mode, night (it winds down),
        /// private (it gives the user privacy) and welcome (it brightens after a
        /// long time away); the chrome's
        /// caption shows `now` (the page on screen and any tab heard) above
        /// `caption` (what moved it over the last 15 minutes) and a count of
        /// the other tabs: quiet ones, which weigh nothing, and untouched
        /// ones, which slow rest.
        Wisp {
            dose: f64,
            phase: WispPhase,
            mode: WispMode,
            trend: WispTrend,
            night: bool,
            private: bool,
            welcome: bool,
            now: Vec<CaptionLine>,
            caption: Vec<CaptionLine>,
            quiet_tabs: u32,
            untouched_tabs: u32,
        },
    }
}

#[cfg(test)]
fn snake_case(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() && i > 0 {
            out.push('_');
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}

/// The contents of `ui/protocol.gen.ts`.
#[cfg(test)]
fn typescript() -> String {
    let declarations = [
        Security::declaration(),
        ChromeView::declaration(),
        TabSound::declaration(),
        WispPhase::declaration(),
        WispMode::declaration(),
        WispTrend::declaration(),
        CaptionKind::declaration(),
        CaptionLine::declaration(),
        DayPart::declaration(),
        PlantKind::declaration(),
        HomeExplain::declaration(),
        HomePlace::declaration(),
        HomeBookmark::declaration(),
        HomePlant::declaration(),
        TimeKind::declaration(),
        DosePoint::declaration(),
        TimeShare::declaration(),
        DiaryDay::declaration(),
        DiaryWeek::declaration(),
        DiarySite::declaration(),
        HomeWisp::declaration(),
        HomeData::declaration(),
        TabInfo::declaration(),
        ToCore::declaration(),
        ToChrome::declaration(),
    ];
    format!(
        "// Generated from core/src/protocol.rs. Do not edit; run scripts/protocol.\n\n{}",
        declarations.join("\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typescript_file_is_current() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../ui/protocol.gen.ts");
        let want = typescript();
        if std::env::var_os("WISP_BLESS").is_some() {
            std::fs::write(path, &want).expect("write ui/protocol.gen.ts");
            return;
        }
        let have = std::fs::read_to_string(path).unwrap_or_default();
        assert!(
            have == want,
            "ui/protocol.gen.ts is out of date; run scripts/protocol"
        );
    }

    #[test]
    fn field_types_map_to_typescript() {
        assert_eq!(<Vec<Option<String>> as Ts>::ts(), "(string | null)[]");
        assert_eq!(<Vec<u32> as Ts>::ts(), "number[]");
        assert_eq!(<Option<bool> as Ts>::ts(), "boolean | null");
    }

    #[test]
    fn chrome_messages_parse() {
        let msg: ToCore = serde_json::from_str(r#"{"type":"navigate","input":"wikipedia.org"}"#)
            .expect("navigate parses");
        assert_eq!(
            msg,
            ToCore::Navigate {
                input: "wikipedia.org".into()
            }
        );
        let msg: ToCore =
            serde_json::from_str(r#"{"type":"ready","view":"toolbar"}"#).expect("ready parses");
        assert_eq!(
            msg,
            ToCore::Ready {
                view: ChromeView::Toolbar
            }
        );
        let msg: ToCore = serde_json::from_str(r#"{"type":"focus_page"}"#).expect("unit parses");
        assert_eq!(msg, ToCore::FocusPage);
    }

    #[test]
    fn unknown_chrome_messages_are_rejected() {
        assert!(serde_json::from_str::<ToCore>(r#"{"type":"format_disk"}"#).is_err());
        assert!(serde_json::from_str::<ToCore>(r#"{"type":"navigate"}"#).is_err());
    }

    #[test]
    fn state_serialises_with_snake_case_tags() {
        let json = serde_json::to_string(&ToChrome::State {
            uri: "https://example.org/".into(),
            title: "Example".into(),
            loading: false,
            progress: 1.0,
            can_go_back: true,
            can_go_forward: false,
            security: Security::NotSecure,
            can_bookmark: true,
            bookmarked: false,
        })
        .expect("state serialises");
        assert!(json.starts_with(r#"{"type":"state","#), "{json}");
        assert!(json.contains(r#""security":"not_secure""#), "{json}");
    }
}
