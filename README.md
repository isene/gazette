<div align="center">

<img src="assets/gazette.svg" width="140" alt="gazette logo">

# gazette

**A terminal reader for your personal daily news digest.**

[![Built with Rust](https://img.shields.io/badge/built%20with-Rust-CE412B?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![Fe2O3 suite](https://img.shields.io/badge/Fe%E2%82%82O%E2%82%83-suite-0b5fa5)](https://github.com/isene/fe2o3)
[![Built on crust](https://img.shields.io/badge/TUI-crust-555)](https://github.com/isene/crust)
[![Release](https://img.shields.io/github/v/release/isene/gazette?color=0b5fa5)](https://github.com/isene/gazette/releases)
[![License: Unlicense](https://img.shields.io/badge/license-Unlicense-3DA639)](https://unlicense.org)
[![Stay Amazing](https://img.shields.io/badge/Stay-Amazing-ff6fa5)](https://isene.org)

</div>

Part of the [Fe₂O₃](https://github.com/isene/fe2o3) Rust terminal suite, built on
[crust](https://github.com/isene/crust).

gazette is a thin reader. The digest itself is generated **server-side** by a
headless Claude run (see *Pipeline* below) and synced to the machine as one
Markdown file per day. gazette lists the available days in a left pane and
renders the selected issue in the right pane.

```
 gazette   2026-06-07   (1/7)              ENTER+N open · j/k scroll · n/p day · q quit
┌────────────┬──────────────────────────────────────────────────────────────────────┐
│ 2026-06-07 │ # News - 2026-06-07                                                    │
│ 2026-06-06 │                                                                        │
│ 2026-06-05 │ ## AI & LLMs                                                           │
│ 2026-06-04 │ ### Anthropic ships Claude Opus 4.8                                    │
│ …          │ Anthropic released Claude Opus 4.8 …                                   │
│            │ [1] https://www.techzine.eu/…                                          │
└────────────┴──────────────────────────────────────────────────────────────────────┘
```

## Keys

| Key | Action |
|---|---|
| `j` / `k` / `↓` / `↑` | scroll the issue |
| `SPACE` / `b` | page down / up |
| `g` / `G` | top / bottom |
| `n` / `p` (also `]` `[`, `→` `←`) | next / previous day |
| `ENTER` then a number | open that `[N]` source link in [scroll](https://github.com/isene/scroll) |
| `r` | reload (re-scan `~/.news`) |
| `q` / `ESC` | quit |

Source URLs are numbered `[N]` inline and also emitted as OSC 8 hyperlinks, so
they are clickable directly in terminals that support it (e.g. glass).

## Data

gazette reads `~/.news/news-YYYY-MM-DD.md` — one issue per day, kept on a 7-day
rolling window. It only ever **reads** that directory; it never writes there.

## Pipeline

The digest is produced by a daily job (e.g. on an always-on server) that runs
headless Claude against a preference prompt and writes the dated Markdown issue
into a synced folder. A minimal generator:

```bash
PROMPT="$(cat ~/.news/news.md)
Today is $(date +%F). Gather the news per the preferences above using web
search, then WRITE the Markdown issue (starting '# News - DATE') to ~/.news/news-$(date +%F).md."
claude -p "$PROMPT" --dangerously-skip-permissions
find ~/.news -name 'news-20*.md' -mtime +7 -delete
```

`news.md` holds your content preferences; edit it to change what the digest
covers. The issue files sync to every device (laptop, phone) via your sync tool
of choice.

## Build

```bash
PATH="/usr/bin:$PATH" cargo build --release
```

`~/bin/gazette` is a symlink to `target/release/gazette`.

## Licence

Unlicense (public domain). Created by Geir Isene.
