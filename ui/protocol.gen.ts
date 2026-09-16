// Generated from core/src/protocol.rs. Do not edit; run scripts/protocol.

export type Security = "secure" | "not_secure" | "local";

export type ChromeView = "sidebar" | "toolbar";

export type TabSound = "silent" | "playing" | "muted";

export type Progress = "running" | "saved" | "stopped" | "failed";

export type Download = { id: number; name: string; path: string; progress: Progress; fraction: number | null };

export type Page = { at: number; url: string; title: string; host: string };

export type WispPhase = "rested" | "engaged" | "clouded" | "drained";

export type WispMode = "away" | "draining" | "resting" | "nourishing" | "holding";

export type WispTrend = "rising" | "falling" | "steady";

export type CaptionKind = "wearing" | "restoring" | "ordinary_sites" | "holding" | "private" | "care" | "away";

export type CaptionLine = { kind: CaptionKind; label: string; bars: number };

export type DayPart = "morning" | "day" | "evening" | "night";

export type PlantKind = "moss" | "fern" | "flower";

export type HomeExplain = { topic: string; text: string };

export type HomeAbout = { title: string; paragraphs: string[]; done: string };

export type HomePlace = { name: string; line: string; url: string; yours: boolean };

export type HomeBookmark = { id: number; title: string; url: string; host: string };

export type HomePlant = { kind: PlantKind; fireflies: boolean };

export type TimeKind = "restoring" | "everyday" | "wearing";

export type DosePoint = { minute: number; dose: number; gap: boolean };

export type TimeShare = { kind: TimeKind; share: number };

export type DiaryDay = { label: string; known: boolean; size: number; shares: TimeShare[]; peak: WispPhase | null; plant: PlantKind | null };

export type DiaryWeek = { label: string; size: number; shares: TimeShare[]; plants: PlantKind[] };

export type DiarySite = { label: string; kind: TimeKind; bars: number };

export type HomeWisp = { today: DosePoint[]; now_minute: number; day_start_minute: number; night_from: number; night_until: number; engaged: number; clouded: number; drained: number; week: DiaryDay[]; earlier: DiaryWeek[]; sites: DiarySite[] };

export type HomeData = { title: string; line: string; about: HomeAbout | null; explain: HomeExplain | null; part: DayPart; places: HomePlace[]; bookmarks: HomeBookmark[]; plants: HomePlant[]; seed: number; wisp: HomeWisp; pages: Page[]; downloads: Download[] };

export type Rating = "drains_a_lot" | "drains_a_little" | "neither" | "restores_a_little" | "restores_a_lot" | "news" | "private" | "unrated" | "care";

export type SiteRating = { site: string; rating: Rating; matched: string; yours: Rating | null; seed: Rating | null };

export type TimeChoice = { value: string; label: string };

export type SettingsData = { lookup: SiteRating[]; lookup_failed: string; ratings: SiteRating[]; ratings_file: string; ratings_problem: string; ask: boolean; night_starts: string; night_ends: string; night_start_choices: TimeChoice[]; night_end_choices: TimeChoice[] };

export type TabInfo = { id: number; title: string; host: string; loading: boolean; sound: TabSound; asleep: boolean };

export type ToCore =
  | { type: "ready"; view: ChromeView }
  | { type: "navigate"; input: string }
  | { type: "navigate_dot_com"; input: string }
  | { type: "back" }
  | { type: "forward" }
  | { type: "reload" }
  | { type: "stop" }
  | { type: "focus_page" }
  | { type: "new_tab" }
  | { type: "close_tab"; id: number }
  | { type: "go_home" }
  | { type: "toggle_bookmark" }
  | { type: "toggle_mute"; id: number }
  | { type: "find"; query: string }
  | { type: "find_next"; backwards: boolean }
  | { type: "close_find" }
  | { type: "select_tab"; id: number }
  | { type: "show_wisp" }
  | { type: "rate_site"; site: string; rating: Rating }
  | { type: "not_now"; site: string }
  | { type: "close_care" }
  | { type: "find_support"; samaritans: boolean }
  | { type: "open_settings" }
  | { type: "open_download"; id: number }
  | { type: "toolbar_layout"; height: number; overlay_height: number; nook_right: number; nook_top: number; nook_width: number; nook_height: number }
  | { type: "minimize" }
  | { type: "toggle_maximize" }
  | { type: "close_window" }
  | { type: "begin_move" };

export type ToChrome =
  | { type: "state"; uri: string; title: string; loading: boolean; progress: number; can_go_back: boolean; can_go_forward: boolean; security: Security; can_bookmark: boolean; bookmarked: boolean }
  | { type: "tabs"; tabs: TabInfo[]; selected: number }
  | { type: "tab_icon"; id: number; icon: string | null }
  | { type: "focus_address" }
  | { type: "downloads"; running: number }
  | { type: "find"; open: boolean }
  | { type: "found"; query: string; summary: string }
  | { type: "window"; floating: boolean }
  | { type: "caption"; open: boolean }
  | { type: "ask"; site: string | null }
  | { type: "care"; open: boolean; samaritans: boolean }
  | { type: "wisp"; dose: number; phase: WispPhase; mode: WispMode; trend: WispTrend; night: boolean; private: boolean; welcome: boolean; now: CaptionLine; caption: CaptionLine[]; other_tabs: number };
