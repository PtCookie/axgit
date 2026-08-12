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
COMPOSE_SHA="$(git -C "$W" rev-parse HEAD)"
GIT_COMMITTER_DATE="2026-07-21T10:00:00+09:00" git -C "$W" notes add -m "Reviewed-by: PtCookie" "$COMPOSE_SHA"
echo "TLS notes" > "$W/NOTES.md"
# A symlink in a subdirectory, targeting its parent — exercises the tree
# listing's `target` field and the web side's relative-path resolution
# (docs/DECISIONS.md #49), neither of which a flat tree would reach.
mkdir -p "$W/docs"
ln -s ../README.md "$W/docs/readme-link"
# A binary file — otherwise unreachable in a local run: the blob/object hex
# dump view (docs/DECISIONS.md #58) and the blob endpoint's binary
# classification both need actual NUL bytes to exercise, not just an
# extension. Written byte-by-byte for reproducibility (no `dd`/random).
printf '\x89PNG\r\n\x1a\n' > "$W/logo.png"
for _ in $(seq 1 32); do
    printf '\x00\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c\x0d\x0e\x0f' >> "$W/logo.png"
done
git -C "$W" add .
commit "$W" "2026-07-22T18:45:00+09:00" "docs: add notes"
GIT_COMMITTER_DATE="2026-07-22T19:00:00+09:00" git -C "$W" tag -a v1.0.0 -m "Release v1.0.0"
# A lightweight tag whose name also contains `/`: the tag detail endpoint's
# lightweight branch (null tag_object/message/tagger) and slash-in-name
# routing are otherwise unreachable in a local run — v1.0.0 above is the
# only annotated tag.
git -C "$W" tag release/0.9 "$COMPOSE_SHA"
# An annotated tag on a blob, not a commit — the only way to reach
# `object.type != "commit"` and `target: null` (cgit's `cgit_object_link`
# case, and the case where the tag has no archive download).
README_BLOB="$(git -C "$W" rev-parse HEAD:README.md)"
GIT_COMMITTER_DATE="2026-07-22T19:05:00+09:00" git -C "$W" tag -a readme-blob -m "Tag pointing at a blob" "$README_BLOB"
# An annotated tag on the root tree — reaches the by-oid object page's tree
# case (`GET /objects/{oid}`, docs/DECISIONS.md #53) from the refs/tag pages
# in a local run, the same way readme-blob reaches the blob case.
ROOT_TREE="$(git -C "$W" rev-parse HEAD^{tree})"
GIT_COMMITTER_DATE="2026-07-22T19:10:00+09:00" git -C "$W" tag -a tree-tag -m "Tag pointing at a tree" "$ROOT_TREE"
git -C "$W" push --quiet "$BARE" main:main \
    refs/tags/v1.0.0 refs/tags/release/0.9 refs/tags/readme-blob refs/tags/tree-tag \
    refs/notes/commits:refs/notes/commits
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

# --- hidden.git: cgit.hide=true — absent from the index, still reachable by
# direct path (docs/DECISIONS.md #66). A local run is the only place this
# distinction is actually visible end to end: `GET /repos` and `GET
# /repos/hidden` both hit the real filesystem scan, unlike the api's own
# tests, which build their fixtures per test.
BARE="$DEST/hidden.git"
git init --quiet --bare --initial-branch=main "$BARE"
git config --file "$BARE/config" cgit.desc "Not listed, but still fetchable"
git config --file "$BARE/config" cgit.hide true

W="$WORK/hidden"
git init --quiet --initial-branch=main "$W"
echo "# hidden" > "$W/README.md"
git -C "$W" add .
commit "$W" "2026-07-15T12:00:00+09:00" "chore: initial commit"
git -C "$W" push --quiet "$BARE" main:main

echo "fixtures ready: $DEST"
