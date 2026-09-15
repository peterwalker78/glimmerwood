// Generated from core/src/protocol.rs. Do not edit; run scripts/protocol.

export type Security = "secure" | "not_secure" | "local";

export type ChromeView = "sidebar" | "toolbar";

export type TabSound = "silent" | "playing" | "muted";

export type WispPhase = "rested" | "engaged" | "clouded" | "drained";

export type WispMode = "away" | "draining" | "resting" | "nourishing" | "holding";

export type WispTrend = "rising" | "falling" | "steady";

export type CaptionKind = "wearing" | "restoring" | "ordinary_sites" | "holding" | "playing" | "private" | "away";

export type CaptionLine = { kind: CaptionKind; label: string; bars: number; heard: boolean };

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

export type HomeData = { title: string; line: string; about: HomeAbout | null; explain: HomeExplain | null; part: DayPart; places: HomePlace[]; bookmarks: HomeBookmark[]; plants: HomePlant[]; seed: number; wisp: HomeWisp };

export type TabInfo = { id: number; title: string; host: string; loading: boolean; sound: TabSound };

export type ToCore =
  | { type: "ready"; view: ChromeView }
  | { type: "navigate"; input: string }
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
  | { type: "find"; open: boolean }
  | { type: "found"; query: string; summary: string }
  | { type: "window"; floating: boolean }
  | { type: "caption"; open: boolean }
  | { type: "wisp"; dose: number; phase: WispPhase; mode: WispMode; trend: WispTrend; night: boolean; private: boolean; welcome: boolean; now: CaptionLine[]; caption: CaptionLine[]; quiet_tabs: number; untouched_tabs: number };
