#!/usr/bin/env bash
# Builds the synthetic bare repositories the README screenshots are taken from
# (docs/DECISIONS.md #99): a handful of invented projects with a branchy
# history, tags and a real README, so the log graph, the summary page and the
# diff view have something to show. Unlike make-fixtures.sh these are for
# looking at, not for tests — nothing asserts on their content.
#
# Usage: scripts/make-screenshot-repos.sh <dest-dir>
# Then:  cargo run --manifest-path api/Cargo.toml -- --repo-root <dest-dir> --static-dir ./web/dist
# Deterministic: fixed dates/authors, host git config ignored.
set -euo pipefail

if [ $# -ne 1 ]; then
    echo "usage: $0 <dest-dir>" >&2
    exit 2
fi
DEST="$1"

export GIT_CONFIG_GLOBAL=/dev/null
export GIT_CONFIG_SYSTEM=/dev/null

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

rm -rf "$DEST"
mkdir -p "$DEST"

MINA_NAME="Mina Seo"
MINA_EMAIL="mina@example.com"
JONAS_NAME="Jonas Weber"
JONAS_EMAIL="jonas@example.com"
PRIYA_NAME="Priya Nair"
PRIYA_EMAIL="priya@example.com"

# who <name> <email> — sets the author and committer for the commands after it
who() {
    export GIT_AUTHOR_NAME="$1" GIT_AUTHOR_EMAIL="$2"
    export GIT_COMMITTER_NAME="$1" GIT_COMMITTER_EMAIL="$2"
}

# commit <work-dir> <date> <message>
commit() {
    git -C "$1" add -A
    GIT_AUTHOR_DATE="$2" GIT_COMMITTER_DATE="$2" git -C "$1" commit --quiet -m "$3"
}

# merge <work-dir> <date> <branch> <message>
merge() {
    GIT_AUTHOR_DATE="$2" GIT_COMMITTER_DATE="$2" git -C "$1" merge --quiet --no-ff -m "$4" "$3"
}

# tag <work-dir> <date> <name>
tag() {
    GIT_COMMITTER_DATE="$2" git -C "$1" tag -a "$3" -m "$3"
}

# publish <work-dir> <bare> <section> <owner> <desc> <last-activity>
# Pushes every branch and tag, then writes the [cgit] metadata and the agefile.
publish() {
    git init --quiet --bare --initial-branch=main "$2"
    git -C "$1" push --quiet "$2" --all
    git -C "$1" push --quiet "$2" --tags
    [ -z "$3" ] || git config --file "$2/config" cgit.section "$3"
    git config --file "$2/config" cgit.owner "$4"
    git config --file "$2/config" cgit.desc "$5"
    mkdir -p "$2/info/web"
    echo "$6" > "$2/info/web/last-modified"
}

# simple <name> <section> <owner> <desc> <date1> <date2>
# A two-commit repository, for the index page.
simple() {
    local w="$WORK/$1"
    git init --quiet --initial-branch=main "$w"
    who "$MINA_NAME" "$MINA_EMAIL"
    echo "# $1" > "$w/README.md"
    commit "$w" "$5" "feat: initial commit"
    echo "$4" >> "$w/README.md"
    commit "$w" "$6" "docs: describe the project"
    publish "$w" "$DEST/$1.git" "$2" "$3" "$4" "${6:0:10} 12:00:00 +0900"
}

# --- lumen: the showcase repository ------------------------------------------
W="$WORK/lumen"
git init --quiet --initial-branch=main "$W"
mkdir -p "$W/src" "$W/themes"

who "$MINA_NAME" "$MINA_EMAIL"
cat > "$W/README.md" <<'EOF'
# Lumen

A fast, friendly static site generator. Write Markdown, get a site.

## Features

- **Fast** — builds a thousand pages in under a second
- **Themes** — drop a CSS file into `themes/` and pick it in `lumen.toml`
- **Live reload** — `lumen serve` rebuilds on every save
- **No runtime** — the output is plain HTML, nothing to host but files

## Quick start

```bash
cargo install lumen
lumen new my-site
cd my-site
lumen serve
```

Pages are Markdown files with an optional front matter block:

```markdown
---
title: Hello, world
date: 2026-08-14
---

Welcome to my site.
```

## Configuration

| Key      | Default   | Description                       |
| -------- | --------- | --------------------------------- |
| `theme`  | `default` | Name of the file under `themes/`  |
| `output` | `public`  | Directory the site is written to  |
| `drafts` | `false`   | Include pages marked `draft: true` |

> Lumen is young. Expect rough edges, and please file what you find.

## License

MIT
EOF
cat > "$W/Cargo.toml" <<'EOF'
[package]
name = "lumen"
version = "0.1.0"
edition = "2024"
EOF
cat > "$W/src/main.rs" <<'EOF'
mod parser;

fn main() {
    let page = parser::parse("# Hello\n\nWelcome.");
    println!("{}", page.html);
}
EOF
cat > "$W/src/parser.rs" <<'EOF'
pub struct Page {
    pub html: String,
}

pub fn parse(source: &str) -> Page {
    let mut html = String::new();
    for line in source.lines() {
        if let Some(title) = line.strip_prefix("# ") {
            html.push_str(&format!("<h1>{title}</h1>\n"));
        } else if !line.is_empty() {
            html.push_str(&format!("<p>{line}</p>\n"));
        }
    }
    Page { html }
}
EOF
commit "$W" "2026-08-10T10:00:00+09:00" "feat: initial commit"

who "$JONAS_NAME" "$JONAS_EMAIL"
git -C "$W" switch --quiet -c feature/themes
cat > "$W/themes/default.css" <<'EOF'
body { font-family: system-ui, sans-serif; max-width: 42rem; margin: 2rem auto; }
EOF
commit "$W" "2026-08-14T15:20:00+09:00" "feat(theme): load a stylesheet from themes/"
cat >> "$W/themes/default.css" <<'EOF'
@media (prefers-color-scheme: dark) { body { background: #111; color: #eee; } }
EOF
commit "$W" "2026-08-16T11:45:00+09:00" "feat(theme): add a dark variant"

who "$MINA_NAME" "$MINA_EMAIL"
git -C "$W" switch --quiet main
cat > "$W/src/parser.rs" <<'EOF'
pub struct Page {
    pub title: Option<String>,
    pub html: String,
}

pub fn parse(source: &str) -> Page {
    let (title, body) = split_front_matter(source);
    let mut html = String::new();
    for line in body.lines() {
        if let Some(heading) = line.strip_prefix("# ") {
            html.push_str(&format!("<h1>{heading}</h1>\n"));
        } else if !line.is_empty() {
            html.push_str(&format!("<p>{line}</p>\n"));
        }
    }
    Page { title, html }
}

fn split_front_matter(source: &str) -> (Option<String>, &str) {
    let Some(rest) = source.strip_prefix("---\n") else {
        return (None, source);
    };
    match rest.split_once("\n---\n") {
        Some((front, body)) => {
            let title = front
                .lines()
                .find_map(|line| line.strip_prefix("title: "))
                .map(str::to_owned);
            (title, body)
        }
        None => (None, source),
    }
}
EOF
commit "$W" "2026-08-18T09:30:00+09:00" "feat(parser): read title from front matter"
who "$JONAS_NAME" "$JONAS_EMAIL"
merge "$W" "2026-08-20T14:00:00+09:00" feature/themes "Merge branch 'feature/themes'"
who "$MINA_NAME" "$MINA_EMAIL"
tag "$W" "2026-08-20T14:10:00+09:00" v0.1.0

who "$PRIYA_NAME" "$PRIYA_EMAIL"
git -C "$W" switch --quiet -c fix/serve-rebuild
cat > "$W/src/main.rs" <<'EOF'
mod parser;
mod watch;

fn main() {
    let page = parser::parse("# Hello\n\nWelcome.");
    println!("{}", page.html);
    watch::on_change(|| println!("rebuilding"));
}
EOF
cat > "$W/src/watch.rs" <<'EOF'
use std::time::Duration;

/// Calls `rebuild` whenever a source file changes, at most once per 200ms.
pub fn on_change(rebuild: impl Fn()) {
    let debounce = Duration::from_millis(200);
    std::thread::sleep(debounce);
    rebuild();
}
EOF
commit "$W" "2026-09-02T17:05:00+09:00" "fix(serve): debounce rebuilds after a save"

who "$MINA_NAME" "$MINA_EMAIL"
git -C "$W" switch --quiet main
cat > "$W/Cargo.toml" <<'EOF'
[package]
name = "lumen"
version = "0.1.0"
edition = "2024"
rust-version = "1.85"
EOF
commit "$W" "2026-09-05T10:15:00+09:00" "build: declare the minimum Rust version"
merge "$W" "2026-09-08T13:30:00+09:00" fix/serve-rebuild "Merge branch 'fix/serve-rebuild'"

# The commit the README's diff screenshot shows: a small, readable refactor.
cat > "$W/src/parser.rs" <<'EOF'
pub struct Page {
    pub title: Option<String>,
    pub html: String,
}

pub fn parse(source: &str) -> Page {
    let (title, body) = split_front_matter(source);
    let html = body
        .lines()
        .filter(|line| !line.is_empty())
        .map(render_line)
        .collect::<String>();
    Page { title, html }
}

fn render_line(line: &str) -> String {
    match line.strip_prefix("# ") {
        Some(heading) => format!("<h1>{heading}</h1>\n"),
        None => format!("<p>{line}</p>\n"),
    }
}

fn split_front_matter(source: &str) -> (Option<String>, &str) {
    let Some(rest) = source.strip_prefix("---\n") else {
        return (None, source);
    };
    match rest.split_once("\n---\n") {
        Some((front, body)) => {
            let title = front
                .lines()
                .find_map(|line| line.strip_prefix("title: "))
                .map(str::to_owned);
            (title, body)
        }
        None => (None, source),
    }
}
EOF
sed -i.bak 's/^- \*\*Fast\*\* — builds a thousand pages in under a second$/- **Fast** — builds a thousand pages in under a second, one pass per page/' "$W/README.md"
rm "$W/README.md.bak"
commit "$W" "2026-09-20T16:40:00+09:00" "refactor(parser): render lines with an iterator chain"
tag "$W" "2026-09-21T09:00:00+09:00" v0.2.0

publish "$W" "$DEST/lumen.git" "" "Mina Seo" "A fast, friendly static site generator" "2026-09-21 09:00:00 +0900"

# --- the rest of the index ---------------------------------------------------
simple tessera "" "Mina Seo" "Tile-based map renderer in Rust" \
    "2026-09-11T10:00:00+09:00" "2026-10-09T18:30:00+09:00"
simple quill "" "Jonas Weber" "A terminal note-taker with fuzzy search" \
    "2026-07-02T10:00:00+09:00" "2026-08-30T20:15:00+09:00"
simple harbor "infra" "Priya Nair" "Container deploy tooling for small servers" \
    "2026-05-04T10:00:00+09:00" "2026-10-01T08:45:00+09:00"
simple compose-kit "infra" "Priya Nair" "Reusable Compose snippets" \
    "2026-03-12T10:00:00+09:00" "2026-06-18T14:20:00+09:00"
simple dotfiles "personal" "Mina Seo" "Shell, editor and terminal configuration" \
    "2025-11-02T10:00:00+09:00" "2026-04-07T22:00:00+09:00"

echo "screenshot repos ready: $DEST"
