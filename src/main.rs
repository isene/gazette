// gazette — terminal reader for the personal daily news digest.
//
// The digest is generated server-side (a headless Claude run on an always-on
// host) and synced here via Syncthing into ~/.news as `news-YYYY-MM-DD.md`,
// one issue per day, 7-day rolling. gazette is a thin reader: a left pane lists
// the available days, the right pane renders the selected issue's Markdown.
//
// Links follow scroll's convention: each source URL is numbered `[N]` inline;
// press ENTER, type the number, ENTER — gazette opens it in `scroll`.
//
// Design goals (Fe2O3): cold when idle (blocking key read, no timers/polling),
// fast startup, minimal work per keystroke.

use crust::{Crust, Pane, Input, style};
use std::path::PathBuf;

const LIST_W: u16 = 14; // left day-list width ("2026-06-07" + marker)

// Palette (xterm-256), aligned with the rest of the suite.
const C_TITLE: u8 = 81; // issue title  (#)
const C_SECTION: u8 = 220; // section     (##)
const C_HEAD: u8 = 255; // item headline (###)
const C_BODY: u8 = 252; // body text
const C_URL: u8 = 240; // url text (dim)
const C_LINKNUM: u8 = 75; // [N] markers (matches scroll's link colour)
const C_SEL: u8 = 81; // selected day

struct Issue {
    date: String,
    path: PathBuf,
}

struct App {
    cols: u16,
    rows: u16,
    top: Pane,
    left: Pane,
    right: Pane,
    foot: Pane,
    issues: Vec<Issue>,
    sel: usize,
    links: Vec<String>, // URLs in the current issue; [N] == links[N-1]
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

/// Render a Markdown issue to styled terminal text and collect its source URLs.
/// Each bare URL line becomes a numbered `[N]` link (also emitted as an OSC 8
/// hyperlink so it stays clickable in glass/kitty). Returns (text, urls).
fn render_md(src: &str) -> (String, Vec<String>) {
    let mut out = String::new();
    let mut urls = Vec::new();
    for raw in src.lines() {
        let line = raw.trim_end();
        if let Some(t) = line.strip_prefix("# ") {
            out.push_str(&style::bold(&style::fg(t, C_TITLE)));
        } else if let Some(t) = line.strip_prefix("## ") {
            out.push('\n');
            out.push_str(&style::bold(&style::fg(t, C_SECTION)));
        } else if let Some(t) = line.strip_prefix("### ") {
            out.push_str(&style::bold(&style::fg(t, C_HEAD)));
        } else if line.starts_with("http://") || line.starts_with("https://") {
            urls.push(line.to_string());
            let n = urls.len();
            let marker = style::fg(&format!("[{}]", n), C_LINKNUM);
            // OSC 8 so the URL is also directly clickable; crust tracks the
            // open hyperlink across wrap/truncate.
            let link = format!("\x1b]8;;{}\x1b\\{}\x1b]8;;\x1b\\", line, line);
            out.push_str(&format!("{} {}", marker, style::fg(&link, C_URL)));
        } else {
            out.push_str(&style::fg(line, C_BODY));
        }
        out.push('\n');
    }
    (out, urls)
}

impl App {
    fn new() -> Self {
        let (cols, rows) = Crust::terminal_size();
        let mut top = Pane::new(1, 1, cols, 1, C_TITLE as u16, 236);
        top.wrap = false;
        top.scroll = false;
        let mut left = Pane::new(1, 2, LIST_W, rows.saturating_sub(2), C_BODY as u16, 0);
        left.wrap = false;
        let mut right = Pane::new(LIST_W + 2, 2, cols.saturating_sub(LIST_W + 1), rows.saturating_sub(2), C_BODY as u16, 0);
        right.wrap = true;
        let mut foot = Pane::new(1, rows, cols, 1, 245, 236);
        foot.wrap = false;
        foot.scroll = false;

        let issues = load_issues();
        let mut app = App { cols, rows, top, left, right, foot, issues, sel: 0, links: Vec::new() };
        app.load_selected();
        app
    }

    /// Load the selected day's issue into the right pane.
    fn load_selected(&mut self) {
        if let Some(issue) = self.issues.get(self.sel) {
            let text = std::fs::read_to_string(&issue.path)
                .unwrap_or_else(|_| "(could not read this issue)".to_string());
            let (styled, urls) = render_md(&text);
            self.right.set_text(&styled);
            self.links = urls;
        } else {
            self.right.set_text(&style::fg(
                "  No news issues yet.\n\n  They appear here once the daily run\n  syncs an issue into ~/.news.",
                C_URL,
            ));
            self.links.clear();
        }
        self.right.ix = 0;
    }

    fn render_top(&mut self) {
        let pos = if self.issues.is_empty() {
            "0/0".to_string()
        } else {
            format!("{}/{}", self.sel + 1, self.issues.len())
        };
        let date = self.issues.get(self.sel).map(|i| i.date.as_str()).unwrap_or("—");
        let title = format!(" gazette   {}   ({})", date, pos);
        let hint = "ENTER+N open · j/k scroll · n/p day · r reload · q quit ";
        let pad = (self.cols as usize)
            .saturating_sub(crust::display_width(&title) + crust::display_width(hint));
        self.top.say(&format!("{}{}{}",
            style::bold(&style::fg(&title, C_TITLE)),
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
        self.right.full_refresh();
        self.render_foot("");
    }

    fn render_foot(&mut self, msg: &str) {
        if msg.is_empty() {
            self.foot.say(&style::fg(
                " Source links are numbered [N] — ENTER then the number opens it in scroll.", 245));
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
        self.right.full_refresh();
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

    fn open_in_scroll(&mut self, url: &str) {
        Crust::cleanup();
        let status = std::process::Command::new("scroll").arg(url).status();
        Crust::init();
        Crust::set_app_identity("gazette");
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
                "j" | "DOWN" => self.right.linedown(),
                "k" | "UP" => self.right.lineup(),
                " " => self.right.pagedown(),
                "b" => self.right.pageup(),
                "g" | "HOME" => self.right.top(),
                "G" | "END" => self.right.bottom(),
                "n" | "]" | "RIGHT" => { let s = self.sel + 1; self.select(s); }
                "p" | "[" | "LEFT" => { let s = self.sel.saturating_sub(1); self.select(s); }
                "ENTER" => self.follow_link(),
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
        println!("gazette — reader for your daily news digest (~/.news/news-*.md)");
        println!("  j/k or arrows  scroll        n/p or [ ]  previous/next day");
        println!("  SPACE/b        page          g/G         top/bottom");
        println!("  ENTER then N   open link [N] in scroll    r reload    q quit");
        return;
    }
    Crust::init();
    Crust::set_app_identity("gazette");
    let mut app = App::new();
    app.run();
    Crust::cleanup();
}
