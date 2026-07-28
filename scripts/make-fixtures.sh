#!/usr/bin/env bash
# Regenerates fixtures/repos/ with a few bare repositories mimicking what the
# git-server container's git-init script produces ([cgit] metadata + agefile).
# Deterministic: fixed dates/authors, host git config ignored.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="$ROOT/fixtures/repos"

export GIT_CONFIG_GLOBAL=/dev/null
export GIT_CONFIG_SYSTEM=/dev/null
export GIT_AUTHOR_NAME="PtCookie"
export GIT_AUTHOR_EMAIL="me@ptcookie.net"
export GIT_COMMITTER_NAME="PtCookie"
export GIT_COMMITTER_EMAIL="me@ptcookie.net"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

rm -rf "$DEST"
mkdir -p "$DEST"

# commit <work-dir> <date> <message>
commit() {
    GIT_AUTHOR_DATE="$2" GIT_COMMITTER_DATE="$2" git -C "$1" commit --quiet -m "$3"
}

# --- git-compose.git: full metadata, 3 commits, tag, agefile -----------------
BARE="$DEST/git-compose.git"
git init --quiet --bare --initial-branch=main "$BARE"
git config --file "$BARE/config" cgit.section infra
git config --file "$BARE/config" cgit.owner PtCookie
git config --file "$BARE/config" cgit.desc "Compose project of Git server"

W="$WORK/git-compose"
git init --quiet --initial-branch=main "$W"
echo "# git-compose" > "$W/README.md"
git -C "$W" add .
commit "$W" "2026-07-20T10:00:00+09:00" "chore: initial commit"
echo "services: {}" > "$W/compose.yaml"
git -C "$W" add .
commit "$W" "2026-07-21T09:30:00+09:00" "feat: add compose file"
echo "TLS notes" > "$W/NOTES.md"
git -C "$W" add .
commit "$W" "2026-07-22T18:45:00+09:00" "docs: add notes"
GIT_COMMITTER_DATE="2026-07-22T19:00:00+09:00" git -C "$W" tag -a v1.0.0 -m "Release v1.0.0"
git -C "$W" push --quiet "$BARE" main:main refs/tags/v1.0.0
mkdir -p "$BARE/info/web"
echo "2026-07-24 13:06:00 +0900" > "$BARE/info/web/last-modified"

# --- dotfiles.git: partial metadata, no agefile (HEAD authordate fallback) ---
BARE="$DEST/dotfiles.git"
git init --quiet --bare --initial-branch=main "$BARE"
git config --file "$BARE/config" cgit.desc "Personal dotfiles"

W="$WORK/dotfiles"
git init --quiet --initial-branch=main "$W"
echo "set -g mouse on" > "$W/tmux.conf"
git -C "$W" add .
commit "$W" "2026-06-01T08:00:00+09:00" "feat: add tmux config"
echo "alias ll='ls -al'" > "$W/aliases.fish"
git -C "$W" add .
commit "$W" "2026-06-15T21:10:00+09:00" "feat: add fish aliases"
git -C "$W" push --quiet "$BARE" main:main

# --- axgit.git: [cgit] and [axgit] both set ([axgit] must win) ---------------
BARE="$DEST/axgit.git"
git init --quiet --bare --initial-branch=main "$BARE"
git config --file "$BARE/config" cgit.section legacy
git config --file "$BARE/config" cgit.desc "cgit description (should lose)"
git config --file "$BARE/config" axgit.section tools
git config --file "$BARE/config" axgit.owner PtCookie
git config --file "$BARE/config" axgit.desc "Axgit web frontend"

W="$WORK/axgit"
git init --quiet --initial-branch=main "$W"
echo "# axgit" > "$W/README.md"
git -C "$W" add .
commit "$W" "2026-07-28T14:00:00+09:00" "feat: initial commit"
git -C "$W" push --quiet "$BARE" main:main
mkdir -p "$BARE/info/web"
echo "2026-07-28 14:00:00 +0900" > "$BARE/info/web/last-modified"

# --- empty.git: no commits, no metadata --------------------------------------
git init --quiet --bare --initial-branch=main "$DEST/empty.git"

echo "fixtures ready: $DEST"
