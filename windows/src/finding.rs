//! Find in page, done inside the tab.
//!
//! WebView2 has no search of its own, so the page is searched by a script
//! run in it. The script has to live beside whatever the page already does:
//! it keeps everything on one property of `window`, marks matches with the
//! highlight API where that exists rather than rearranging the document, and
//! puts back exactly what it found on the way out.

/// What the script is asked to do, ready to hand to `ExecuteScript`. The
/// answer is the number of matches, which the find bar shows in words.
pub fn search(query: &str) -> String {
    doing(&format!("return search({});", quoted(query)))
}

/// The next match along, or the one before. The answer is the same count.
pub fn step(backwards: bool) -> String {
    doing(&format!("return step({backwards});"))
}

/// Everything back as it was, and nothing of ours left on the page.
pub fn close() -> String {
    doing("return wipe(true);")
}

fn doing(tail: &str) -> String {
    SCRIPT.replace("/*ACTION*/", tail)
}

/// A string written the way JavaScript would write it.
fn quoted(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| String::from("\"\""))
}

/// The search itself. It is handed to the page whole every time, because a
/// page may have been replaced since the last time, and it leaves behind
/// only the one property it keeps its matches on.
const SCRIPT: &str = r#"(() => {
  // The one name this adds to the page, and the two it registers
  // highlights under. Nothing a page is likely to carry is called any of
  // these, and nothing else of ours is left anywhere.
  const KEY = "__glimmerwoodFind_4a7e2c";
  const QUIET = "glimmerwood-find-4a7e2c";
  const LOUD = "glimmerwood-find-at-4a7e2c";
  // How many matches are marked and stepped through. The count reported is
  // the true one, so the bar can say there are more than this.
  const CAP = 999;
  // Stands between one block of text and the next, so that a search doesn't
  // run out of the end of one paragraph into the start of another. Nothing
  // typed into the find bar can contain it.
  const BREAK = "\u0000";

  const QUIET_INK = "background-color:#ffe066;color:#000";
  const LOUD_INK = "background-color:#ff9632;color:#000";

  // Marking without touching the document at all, where the engine has it.
  const marks =
    typeof CSS !== "undefined" &&
    typeof CSS.highlights !== "undefined" &&
    typeof Highlight === "function";

  const state =
    window[KEY] ||
    (window[KEY] = {
      query: "",
      ranges: [],
      wraps: [],
      count: 0,
      at: 0,
      sheet: null,
      style: null,
    });

  // Text in these is not text the reader is reading.
  const PASS = new Set([
    "SCRIPT", "STYLE", "NOSCRIPT", "TEMPLATE", "TITLE", "HEAD",
    "IFRAME", "OBJECT", "SELECT", "OPTION", "DATALIST",
  ]);
  // Where one run of text ends and the next begins.
  const BLOCKS = new Set([
    "ADDRESS", "ARTICLE", "ASIDE", "BLOCKQUOTE", "BR", "CAPTION", "DD",
    "DETAILS", "DIALOG", "DIV", "DL", "DT", "FIELDSET", "FIGCAPTION",
    "FIGURE", "FOOTER", "FORM", "H1", "H2", "H3", "H4", "H5", "H6",
    "HEADER", "HR", "LI", "MAIN", "NAV", "OL", "P", "PRE", "SECTION",
    "SUMMARY", "TABLE", "TD", "TEXTAREA", "TH", "TR", "UL",
  ]);

  // Colours the two highlights and nothing else: a page with highlights of
  // its own keeps whatever it gives them.
  const rule =
    "::highlight(" + QUIET + "){" + QUIET_INK + "}" +
    "::highlight(" + LOUD + "){" + LOUD_INK + "}";

  function inkOn() {
    if (state.sheet || state.style) return;
    try {
      const sheet = new CSSStyleSheet();
      sheet.replaceSync(rule);
      document.adoptedStyleSheets = document.adoptedStyleSheets.concat(sheet);
      state.sheet = sheet;
      return;
    } catch {
      // A document that won't take a sheet of that kind gets an element
      // instead, which is taken away again on the way out.
    }
    const style = document.createElement("style");
    style.textContent = rule;
    (document.head || document.documentElement).append(style);
    state.style = style;
  }

  function inkOff() {
    if (state.sheet) {
      try {
        document.adoptedStyleSheets = document.adoptedStyleSheets.filter(
          (sheet) => sheet !== state.sheet
        );
      } catch {}
      state.sheet = null;
    }
    if (state.style) {
      state.style.remove();
      state.style = null;
    }
  }

  // Everything the page is showing, as one string, with the text nodes each
  // stretch of it came from.
  function reading() {
    const root = document.body || document.documentElement;
    if (!root) return { raw: "", parts: [] };
    const walker = document.createTreeWalker(
      root,
      NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT,
      {
        acceptNode(node) {
          if (node.nodeType === Node.TEXT_NODE) {
            return node.nodeValue
              ? NodeFilter.FILTER_ACCEPT
              : NodeFilter.FILTER_REJECT;
          }
          if (PASS.has(node.nodeName)) return NodeFilter.FILTER_REJECT;
          if (
            node.checkVisibility &&
            !node.checkVisibility({ visibilityProperty: true })
          ) {
            return NodeFilter.FILTER_REJECT;
          }
          return NodeFilter.FILTER_ACCEPT;
        },
      }
    );
    let raw = "";
    const parts = [];
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      if (node.nodeType === Node.TEXT_NODE) {
        parts.push({ node: node, from: raw.length });
        raw += node.nodeValue;
      } else if (BLOCKS.has(node.nodeName)) {
        raw += BREAK;
      }
    }
    return { raw: raw, parts: parts };
  }

  // The same text with the runs of space that HTML source leaves behind
  // closed up, keeping which character of the original each one was.
  function closedUp(raw) {
    let text = "";
    const at = [];
    let spacing = true;
    for (let i = 0; i < raw.length; i++) {
      const ch = raw[i];
      if (ch === BREAK) {
        if (text && text[text.length - 1] !== BREAK) {
          text += BREAK;
          at.push(i);
        }
        spacing = true;
      } else if (
        ch === " " || ch === "\n" || ch === "\t" ||
        ch === "\r" || ch === "\f" || ch === "\u00a0"
      ) {
        if (!spacing) {
          spacing = true;
          text += " ";
          at.push(i);
        }
      } else {
        spacing = false;
        text += ch;
        at.push(i);
      }
    }
    return { text: text, at: at };
  }

  // Lower case, a character at a time wherever doing it wholesale would
  // change the length and lose track of where a match is.
  function folded(text) {
    const lower = text.toLowerCase();
    if (lower.length === text.length) return lower;
    let out = "";
    for (const ch of text) {
      const one = ch.toLowerCase();
      out += one.length === ch.length ? one : ch;
    }
    return out;
  }

  // Which text node a place in the gathered text fell in.
  function within(parts, where) {
    let low = 0;
    let high = parts.length - 1;
    while (low <= high) {
      const mid = (low + high) >> 1;
      const part = parts[mid];
      if (where < part.from) {
        high = mid - 1;
      } else if (where >= part.from + part.node.nodeValue.length) {
        low = mid + 1;
      } else {
        return part;
      }
    }
    return null;
  }

  function rangeOf(parts, from, to) {
    const start = within(parts, from);
    const end = within(parts, to - 1);
    if (!start || !end) return null;
    const range = document.createRange();
    try {
      range.setStart(start.node, from - start.from);
      range.setEnd(end.node, to - end.from);
    } catch {
      return null;
    }
    return range;
  }

  function gather(query) {
    state.ranges = [];
    state.count = 0;
    const read = reading();
    const flat = closedUp(read.raw);
    const hay = folded(flat.text);
    const needle = folded(query).replace(/\s+/g, " ");
    if (!needle) return;
    for (let from = 0; ; ) {
      const found = hay.indexOf(needle, from);
      if (found < 0) break;
      state.count++;
      if (state.ranges.length < CAP) {
        const range = rangeOf(
          read.parts,
          flat.at[found],
          flat.at[found + needle.length - 1] + 1
        );
        if (range) state.ranges.push(range);
      }
      from = found + needle.length;
    }
  }

  function paint() {
    if (!state.ranges.length) {
      if (marks) {
        CSS.highlights.delete(QUIET);
        CSS.highlights.delete(LOUD);
      }
      return;
    }
    const many = state.ranges.length;
    state.at = ((state.at % many) + many) % many;
    const here = state.ranges[state.at];
    if (marks) {
      inkOn();
      const rest = state.ranges.filter((range, i) => i !== state.at);
      CSS.highlights.set(QUIET, new Highlight(...rest));
      CSS.highlights.set(LOUD, new Highlight(here));
    } else {
      wrap();
    }
    reveal(here);
  }

  // What is left when the engine has no highlight API: a span around each
  // match, carrying its colour itself so no stylesheet is needed.
  function wrap() {
    // Back to front, so that marking one match doesn't disturb the place of
    // the ones still to be marked.
    for (let i = state.ranges.length - 1; i >= 0; i--) {
      const span = document.createElement("span");
      span.setAttribute("style", i === state.at ? LOUD_INK : QUIET_INK);
      try {
        state.ranges[i].surroundContents(span);
        state.wraps.push(span);
      } catch {
        // A match running across the edge of an element can't be put inside
        // one span; it goes unmarked rather than the page being rearranged.
      }
    }
  }

  // Bring a match into view, and only when it isn't in view already.
  function reveal(range) {
    const tall = window.innerHeight || document.documentElement.clientHeight;
    const wide = window.innerWidth || document.documentElement.clientWidth;
    const box = range.getBoundingClientRect();
    if (box.top >= 0 && box.bottom <= tall && box.left >= 0 && box.right <= wide) {
      return;
    }
    const holder = range.startContainer.parentElement;
    if (holder) holder.scrollIntoView({ block: "center", inline: "nearest" });
    // Scrolling the element that holds it need not have brought the match
    // itself into the window.
    const now = range.getBoundingClientRect();
    if (now.top < 0 || now.bottom > tall) {
      window.scrollBy(0, now.top - tall / 2);
    }
  }

  // Take it all back: our highlights, our elements, and the text nodes the
  // page started with. `done` is the find bar closing, after which there is
  // nothing of ours on the page at all.
  function wipe(done) {
    if (marks) {
      // The two we registered and no others: a page keeping highlights of
      // its own keeps them.
      CSS.highlights.delete(QUIET);
      CSS.highlights.delete(LOUD);
    }
    for (const span of state.wraps) {
      const parent = span.parentNode;
      if (!parent) continue;
      parent.replaceChild(document.createTextNode(span.textContent), span);
      // Puts the text node back as one, the way it was before it was split.
      parent.normalize();
    }
    state.wraps = [];
    state.ranges = [];
    state.count = 0;
    state.at = 0;
    inkOff();
    if (done) {
      state.query = "";
      delete window[KEY];
    }
    return null;
  }

  function search(query) {
    wipe(false);
    state.query = query;
    if (!query) return 0;
    gather(query);
    state.at = 0;
    paint();
    return state.count;
  }

  function step(backwards) {
    if (!state.query) return 0;
    const want = state.at + (backwards ? -1 : 1);
    if (!marks) {
      // The marks are elements here, and taking them away joins the text
      // nodes back together, which leaves the ranges pointing nowhere.
      wipe(false);
      gather(state.query);
    }
    state.at = want;
    paint();
    return state.count;
  }

  /*ACTION*/
})()"#;
