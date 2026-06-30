// gazette — terminal reader for the personal daily news digest.
//
// The digest is generated server-side (a headless Claude run on an always-on
// host) and synced here via Syncthing into ~/.news as `news-YYYY-MM-DD.md`,
// one issue per day, 7-day rolling. gazette is a thin reader: a left pane lists
// the available days, the right pane renders the selected issue's Markdown as a
// two-column newspaper spread.
//
// Layout: each `## section` is kept together — a section that starts in a
// column runs all the way down that column and is never split across the
// column break (the right column may end up shorter, which is fine). The
// content is paginated into two-column spreads; PgDn / PgUp turn a page.
//
// Links follow scroll's convention: each source URL is numbered `[N]` and shown
// as `[N] hostname` (not the full URL); press ENTER, type the number, ENTER —
// gazette opens it in `scroll`. The label is also an OSC 8 hyperlink, so it is
// directly clickable in glass.
//
// Design goals (Fe2O3): cold when idle (blocking key read, no timers/polling),
// fast startup, minimal work per keystroke.

use crust::{Crust, Pane, Input, style};
use std::path::PathBuf;

const LIST_W: u16 = 14; // left day-list width ("2026-06-07" + marker)
const GUTTER: usize = 3; // space between the two reading columns

// Palette (xterm-256), aligned with the rest of the suite.
const C_SECTION: u8 = 220; // section     (##)
const C_HEAD: u8 = 255; // item headline (###)
const C_BODY: u8 = 252; // body text
const C_URL: u8 = 245; // url host (dim)
const C_LINKNUM: u8 = 75; // [N] markers (matches scroll's link colour)
const C_SEP: u8 = 238; // section rule
const C_SEL: u8 = 81; // selected day

struct Issue {
    date: String,
    path: PathBuf,
}

struct App {
    cols: u16,
    top: Pane,
    left: Pane,
    right: Pane,
    foot: Pane,
    issues: Vec<Issue>,
    sel: usize,
    links: Vec<String>, // full URLs in the current issue; [N] == links[N-1]
    columns: Vec<Vec<String>>, // section-kept columns, each col_h lines tall
    page: usize,        // current two-column spread (shows columns 2p, 2p+1)
    col_w: usize,       // one column's text width
    col_h: usize,       // one column's height in rows
}

fn news_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".news")
}

/// Scan ~/.news for `news-YYYY-MM-DD.md`, newest first.
fn load_issues() -> Vec<Issue> {
    let mut v = Vec::new();
    if let Ok(rd) = std::fs::read_dir(news_dir()) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if let Some(rest) = name.strip_prefix("news-") {
                if let Some(date) = rest.strip_suffix(".md") {
                    v.push(Issue { date: date.to_string(), path: e.path() });
                }
            }
        }
    }
    v.sort_by(|a, b| b.date.cmp(&a.date));
    v
}

/// Greedy word-wrap of plain text to `width`, hard-splitting any word longer
/// than the column. Returns at least one (possibly empty) line.
fn wrap_plain(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    let mut line = String::new();
    let mut line_w = 0usize;
    for word in text.split_whitespace() {
        let w = crust::display_width(word);
        if w > width {
            if !line.is_empty() {
                out.push(std::mem::take(&mut line));
            }
            let mut chunk = String::new();
            let mut cw = 0;
            for ch in word.chars() {
                let cwid = crust::display_width(&ch.to_string());
                if cw + cwid > width {
                    out.push(std::mem::take(&mut chunk));
                    cw = 0;
                }
                chunk.push(ch);
                cw += cwid;
            }
            line = chunk;
            line_w = cw;
            continue;
        }
        if line_w == 0 {
            line.push_str(word);
            line_w = w;
        } else if line_w + 1 + w <= width {
            line.push(' ');
            line.push_str(word);
            line_w += 1 + w;
        } else {
            out.push(std::mem::take(&mut line));
            line.push_str(word);
            line_w = w;
        }
    }
    out.push(line);
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// `https://www.techzine.eu/news/...` → `techzine.eu` (scheme + leading www.
/// stripped, path dropped). Keeps link rows short and scannable.
fn hostname(url: &str) -> String {
    let after = url.split("://").nth(1).unwrap_or(url);
    let host = after.split('/').next().unwrap_or(after);
    host.strip_prefix("www.").unwrap_or(host).to_string()
}

/// Close the current section: drop trailing blank lines and, if any content
/// remains, move it into `sections`.
fn flush_section(cur: &mut Vec<String>, sections: &mut Vec<Vec<String>>) {
    while matches!(cur.last(), Some(s) if s.is_empty()) { cur.pop(); }
    if !cur.is_empty() { sections.push(std::mem::take(cur)); }
}

/// Split an issue's Markdown into SECTIONS of styled, column-width-wrapped
/// lines. A new section starts at each `##`; its `###` items (with bodies and
/// source links, one blank line between items) all belong to that section. The
/// section is the keep-together unit — laid whole into a single column.
fn issue_sections(md: &str, col_w: usize, links: &mut Vec<String>) -> Vec<Vec<String>> {
    links.clear();
    let mut sections: Vec<Vec<String>> = Vec::new();
    let mut cur: Vec<String> = Vec::new();
    let mut first_item = true; // is the next `###` the first item of this section?
    for raw in md.lines() {
        let line = raw.trim_end();
        if line.starts_with("# ") {
            continue; // title is shown in the top bar
        } else if let Some(t) = line.strip_prefix("## ") {
            flush_section(&mut cur, &mut sections);
            for sub in wrap_plain(t, col_w) { cur.push(style::bold(&style::fg(&sub, C_SECTION))); }
            cur.push(style::fg(&"\u{2500}".repeat(col_w), C_SEP));
            first_item = true;
        } else if let Some(t) = line.strip_prefix("### ") {
            if !first_item { cur.push(String::new()); } // blank between items
            for sub in wrap_plain(t, col_w) { cur.push(style::bold(&style::fg(&sub, C_HEAD))); }
            first_item = false;
        } else if line.starts_with("http://") || line.starts_with("https://") {
            links.push(line.to_string());
            let n = links.len();
            let host = hostname(line);
            // OSC 8 around the label so a glass click opens the full URL too.
            let labelled = format!("\x1b]8;;{}\x1b\\{}\x1b]8;;\x1b\\", line, style::fg(&host, C_URL));
            cur.push(format!("{} {}", style::fg(&format!("[{}]", n), C_LINKNUM), labelled));
        } else if line.is_empty() {
            // ignored — item spacing is added at the `###` boundary
        } else {
            for sub in wrap_plain(line, col_w) { cur.push(style::fg(&sub, C_BODY)); }
        }
    }
    flush_section(&mut cur, &mut sections);
    sections
}

/// Pad `cur` to exactly `h` lines and push it as a finished column.
fn push_column(cur: &mut Vec<String>, cols: &mut Vec<Vec<String>>, h: usize) {
    while cur.len() < h { cur.push(String::new()); }
    cols.push(std::mem::take(cur));
}

/// Lay sections into fixed-height columns, keeping each section together: a
/// section that does not fit in the current column's remaining space starts a
/// fresh column (leaving the previous column short), so it is never split
/// across the column break. A section taller than a whole column (rare on a
/// normal terminal) is the only thing allowed to flow across columns.
fn layout_columns(sections: Vec<Vec<String>>, h: usize) -> Vec<Vec<String>> {
    let h = h.max(1);
    let mut cols: Vec<Vec<String>> = Vec::new();
    let mut cur: Vec<String> = Vec::new();
    for s in sections {
        let sep = if cur.is_empty() { 0 } else { 1 }; // blank line between sections
        if cur.len() + sep + s.len() <= h {
            if sep == 1 { cur.push(String::new()); }
            cur.extend(s);
        } else if s.len() <= h {
            push_column(&mut cur, &mut cols, h);
            cur.extend(s);
        } else {
            if !cur.is_empty() { push_column(&mut cur, &mut cols, h); }
            for line in s {
                if cur.len() == h { cols.push(std::mem::take(&mut cur)); }
                cur.push(line);
            }
        }
    }
    if !cur.is_empty() { push_column(&mut cur, &mut cols, h); }
    cols
}

impl App {
    fn new() -> Self {
        let (cols, rows) = Crust::terminal_size();
        let mut top = Pane::new(1, 1, cols, 1, 81, 236);
        top.wrap = false;
        top.scroll = false;
        // Content panes start at row 3 (a blank "breathing" row below the top
        // bar) and end one row above the foot, so the reading area has a little
        // air top and bottom.
        let body_h = rows.saturating_sub(4);
        let mut left = Pane::new(1, 3, LIST_W, body_h, C_BODY as u16, 0);
        left.wrap = false;
        let mut right = Pane::new(LIST_W + 2, 3, cols.saturating_sub(LIST_W + 1), body_h, C_BODY as u16, 0);
        right.wrap = false;
        right.scroll = false;
        let mut foot = Pane::new(1, rows, cols, 1, 245, 236);
        foot.wrap = false;
        foot.scroll = false;

        let issues = load_issues();
        let mut app = App {
            cols, top, left, right, foot, issues, sel: 0,
            links: Vec::new(), columns: Vec::new(), page: 0, col_w: 0, col_h: 0,
        };
        app.load_selected();
        app
    }

    /// Read the selected issue, wrap + paginate into section-kept columns,
    /// reset to the first page.
    fn load_selected(&mut self) {
        self.col_w = (self.right.w as usize).saturating_sub(GUTTER) / 2;
        self.col_h = self.right.h as usize;
        self.page = 0;
        let sections = if let Some(issue) = self.issues.get(self.sel) {
            let text = std::fs::read_to_string(&issue.path)
                .unwrap_or_else(|_| "(could not read this issue)".to_string());
            issue_sections(&text, self.col_w, &mut self.links)
        } else {
            self.links.clear();
            vec![vec![
                style::fg("No news issues yet.", C_URL),
                String::new(),
                style::fg("They appear here once the daily", C_URL),
                style::fg("run syncs an issue into ~/.news.", C_URL),
            ]]
        };
        self.columns = layout_columns(sections, self.col_h);
    }

    /// Index of the last two-column spread.
    fn last_page(&self) -> usize {
        self.columns.len().saturating_sub(1) / 2
    }

    /// Paint the right pane as the current two-column spread.
    fn render_right(&mut self) {
        let h = self.col_h.max(1);
        let cw = self.col_w;
        let left = self.columns.get(2 * self.page);
        let right = self.columns.get(2 * self.page + 1);
        let mut frame = String::new();
        for i in 0..h {
            let l = left.and_then(|c| c.get(i)).map(|s| s.as_str()).unwrap_or("");
            let r = right.and_then(|c| c.get(i)).map(|s| s.as_str()).unwrap_or("");
            let pad = cw.saturating_sub(crust::display_width(l)) + GUTTER;
            frame.push_str(l);
            if !r.is_empty() {
                frame.push_str(&" ".repeat(pad));
                frame.push_str(r);
            }
            if i + 1 < h {
                frame.push('\n');
            }
        }
        self.right.set_text(&frame);
        self.right.ix = 0;
        self.right.full_refresh();
    }

    fn render_top(&mut self) {
        let day = if self.issues.is_empty() {
            "0/0".to_string()
        } else {
            format!("{}/{}", self.sel + 1, self.issues.len())
        };
        let date = self.issues.get(self.sel).map(|i| i.date.as_str()).unwrap_or("\u{2014}");
        let pages = self.last_page() + 1;
        let title = format!(" gazette   {}   (day {} \u{00b7} p{}/{})", date, day, self.page + 1, pages);
        let hint = "ENTER+N open \u{00b7} ^A discuss \u{00b7} PgUp/Dn \u{00b7} n/p day \u{00b7} q quit ";
        let pad = (self.cols as usize)
            .saturating_sub(crust::display_width(&title) + crust::display_width(hint));
        self.top.say(&format!("{}{}{}",
            style::bold(&style::fg(&title, 81)),
            " ".repeat(pad),
            style::fg(hint, 245)));
    }

    fn render_left(&mut self) {
        let mut lines = String::new();
        for (i, issue) in self.issues.iter().enumerate() {
            if i == self.sel {
                lines.push_str(&style::reverse(&style::fg(&format!(" {} ", issue.date), C_SEL)));
            } else {
                lines.push_str(&style::fg(&format!(" {}", issue.date), C_BODY));
            }
            lines.push('\n');
        }
        self.left.set_text(&lines);
        self.left.full_refresh();
    }

    fn render_all(&mut self) {
        self.render_top();
        self.render_left();
        self.render_right();
        self.render_foot("");
    }

    fn render_foot(&mut self, msg: &str) {
        if msg.is_empty() {
            self.foot.say(&style::fg(
                " Links are numbered [N] \u{2014} ENTER then the number opens it in scroll (or click in glass).", 245));
        } else {
            self.foot.say(&style::fg(msg, C_SECTION));
        }
    }

    fn select(&mut self, new: usize) {
        if self.issues.is_empty() || new == self.sel {
            return;
        }
        self.sel = new.min(self.issues.len() - 1);
        self.load_selected();
        self.render_top();
        self.render_left();
        self.render_right();
    }

    /// Turn to a page (clamped), repainting if it changed.
    fn goto_page(&mut self, p: usize) {
        let clamped = p.min(self.last_page());
        if clamped != self.page {
            self.page = clamped;
            self.page_changed();
        }
    }

    fn page_changed(&mut self) {
        self.render_top();
        self.render_right();
    }

    /// ENTER: prompt for a link number (scroll's convention) and open it in
    /// scroll. Suspends gazette's screen while scroll runs, then restores.
    fn follow_link(&mut self) {
        if self.links.is_empty() {
            self.render_foot(" No links in this issue.");
            return;
        }
        let input = self.foot.ask("Link #: ", "");
        let trimmed = input.trim();
        if trimmed.is_empty() {
            self.render_foot("");
            return;
        }
        let Ok(n) = trimmed.parse::<usize>() else {
            self.render_foot(&format!(" Invalid link number: {}", trimmed));
            return;
        };
        let Some(url) = self.links.get(n.wrapping_sub(1)).cloned() else {
            self.render_foot(&format!(" Link {} not found (1-{}).", n, self.links.len()));
            return;
        };
        self.open_in_scroll(&url);
    }

    /// `c` — open an interactive Claude session seeded with the whole current
    /// issue (full text + every source URL), to discuss the news and go deeper.
    /// Suspends gazette while claude runs, then restores it (same alt-screen
    /// dance as open_in_scroll). Cold when idle — fires only on the keypress.
    fn discuss_news(&mut self) {
        let Some(issue) = self.issues.get(self.sel) else {
            self.render_foot(" No issue to discuss.");
            return;
        };
        let date = issue.date.clone();
        let body = std::fs::read_to_string(&issue.path).unwrap_or_default();
        if body.trim().is_empty() {
            self.render_foot(" This issue is empty \u{2014} nothing to discuss.");
            return;
        }
        let prompt = format!(
            "Let's discuss my news digest for {date}. Here is the full issue, \
             including every source URL:\n\n{body}\n\nGive me a brief overview \
             of the most notable items, then let's talk \u{2014} I'll ask about \
             specific stories and you can go deeper, pull up the linked sources, \
             and add context."
        );
        Crust::cleanup();
        let status = std::process::Command::new("claude").arg(&prompt).status();
        Crust::init();
        Crust::set_app_identity("gazette");
        self.top.invalidate();
        self.foot.invalidate();
        self.render_all();
        if status.is_err() {
            self.render_foot(" Could not launch 'claude' (is it on PATH?).");
        }
    }

    fn open_in_scroll(&mut self, url: &str) {
        Crust::cleanup();
        let status = std::process::Command::new("scroll").arg(url).status();
        Crust::init();
        Crust::set_app_identity("gazette");
        // Leaving + re-entering the alt screen around scroll wipes the screen,
        // but the diff-cached say() panes (top, foot) still hold pre-scroll
        // content and would skip repainting identical text — leaving row 1 /
        // the foot blank. Invalidate them so render_all repaints. (left/right
        // use full_refresh, which self-clears its diff cache.)
        self.top.invalidate();
        self.foot.invalidate();
        self.render_all();
        if status.is_err() {
            self.render_foot(" Could not launch 'scroll' (is it on PATH?).");
        }
    }

    fn run(&mut self) {
        self.render_all();
        loop {
            let Some(key) = Input::getchr(None) else { continue };
            match key.as_str() {
                "q" | "ESC" => break,
                // Section keep-together needs a fixed spread, so navigation is
                // by page (two columns at a time), not by line.
                "j" | "DOWN" | " " | "PgDOWN" => self.goto_page(self.page + 1),
                "k" | "UP" | "b" | "PgUP" => self.goto_page(self.page.saturating_sub(1)),
                "g" | "HOME" => self.goto_page(0),
                "G" | "END" => self.goto_page(self.last_page()),
                "n" | "]" | "RIGHT" => { let s = self.sel + 1; self.select(s); }
                "p" | "[" | "LEFT" => { let s = self.sel.saturating_sub(1); self.select(s); }
                "ENTER" => self.follow_link(),
                "C-A" => self.discuss_news(), // Fe2O3-standard: full CC session
                "r" => {
                    self.issues = load_issues();
                    if self.sel >= self.issues.len() { self.sel = self.issues.len().saturating_sub(1); }
                    self.load_selected();
                    self.render_all();
                    self.render_foot(" Reloaded.");
                }
                _ => {}
            }
        }
    }
}

fn main() {
    // --help / -h before entering the alt screen.
    if std::env::args().skip(1).any(|a| a == "-h" || a == "--help") {
        println!("gazette \u{2014} reader for your daily news digest (~/.news/news-*.md)");
        println!("  PgDn/PgUp (or j/k, arrows, SPACE/b)   turn a two-column page");
        println!("  g/Home  G/End   first / last page     n/p or [ ]    prev/next day");
        println!("  ENTER then N    open link [N] in scroll");
        println!("  Ctrl-A  discuss the issue with Claude (full text + links)    r reload   q quit");
        return;
    }
    Crust::init();
    Crust::set_app_identity("gazette");
    let mut app = App::new();
    app.run();
    Crust::cleanup();
}
