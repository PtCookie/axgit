# Axgit Roadmap

What's left to build, and the cgit-parity decisions that are deliberately settled. Completed work
is not tracked here — the reasoning behind each shipped feature lives in `docs/DECISIONS.md`
(cited from source comments as `docs/DECISIONS.md #NN`), and the sequence of changes is in the git
history. When finishing a piece of work, replace the "Next up" section with the next target and
add its decision entry to `docs/DECISIONS.md`, in the same commit.

## Next up

**Nothing is queued.** The e2e suite now fails a test whose page requested an `/api` endpoint the
spec never stubbed (#96), instead of letting it leak to the dev proxy and pass against an error
fallback. Before that, syntax highlighting themes became configurable per color mode (#95),
which closed the last piece of site presentation still hardcoded — everything an operator sees
is now reachable from flags, `AXGIT_*` env vars or the config file. Before that, the GitHub
Actions migration (#92) closed the last item that had a deadline attached: CI runs on every push
to `main` and on pull requests, and a `v*` tag publishes both the release tarballs and a
multi-platform GHCR image — the CD half the Jenkins pipeline never had. No cgit-parity gaps
remain (single-child directory collapsing was deliberately left unimplemented, see the "not
planned" notes below), the single-binary deploy path — feature (#74) and packaging (#75) both —
is done, configuration has a file surface as well as flags/env (#87), and the docs live next to
the code they describe: the API contract and backend design in `api/README.md`, the frontend
design in `web/README.md`, deployment in `README.md` (#89, #90).

Pick the next piece of work from the candidates below.

### Candidates (not urgent, no particular order)

- `git grep`/`git log` exec fallbacks for search/stats if either proves too slow on a large
  repository — both left this escape hatch for themselves (DECISIONS.md #26/#28).
- `follow=1` on the stats `path` filter, tracking renames the same way `/commits` does (#56) — left
  out of #59 on purpose (cgit's stats page doesn't track renames either); revisit if a real need
  shows up.
- Commit log's `path` filter walk can be slow on paths that change rarely across a long history
  (noted when `commits.rs::log` was built) — no reports of this being a real problem yet.
- Tree/blob/blame links for remote branches on the refs page (#52 built Log/Compare only) — needs
  `resolve.rs::ref_shorthands()` extended to `refs/remotes/*`, plus deciding how a local branch
  named e.g. `origin` should disambiguate against a remote branch `origin/main` under the existing
  longest-match rule. No confirmed need yet (#52's own investigation found no repository that
  actually populates `remote_branches`).

### cgit parity gaps (from a cgit feature audit)

Found by walking cgit's `cmd.c` dispatch table (21 commands) and `cgitrc.5.txt`'s full option list
against axgit's routes and pages. Not urgent, no particular order — pick from here the same way as
the candidates above. Items that turned out to be merged, or built differently on purpose, are
recorded separately below instead of listed as gaps.

(none remaining)

### cgit parity notes (merged or deliberately different — not planned)

Not gaps — recorded so a future session doesn't rediscover these as missing and try to rebuild
them. Grouped by why the difference exists.

**Merged into one axgit feature**

- cgit's `about` page → the body column of `/{repo}` summary (#21, #31). cgit's summary page also
  lists branches/tags/recent-log; axgit shows counts only and defers to the Refs/Log tabs.
- cgit's `plain` + `blob` → one `/raw/{ref}/{path...}`.
- cgit's three log search modes (`qt=grep|author|committer`) plus index search → one
  `/search?type=` and a single Search tab (#26, #27) — cgit's own search UI presents them as one
  box with a type selector too, which is the explicit rationale.
- cgit's `snapshot` → the equivalently-named `/archive` (naming only). The filename-guesses-the-ref
  DWIM behavior is not reproduced (#35).
- Caching: cgit's on-disk slots + pure TTL → an in-process moka cache with a HEAD/agefile
  validator plus ETag (#6). cgit serves stale responses until the TTL lapses even after a push; no
  analogue of `ls_cache` is needed.

**Built differently on purpose**

- Stats window anchored on the resolved commit's authordate rather than request time (for
  immutable caching), and 12 buckets + a bar chart instead of a 4-bucket table (#28, #29).
- Stats' `path` filter has no `follow` — unlike `/commits`, it never tracks a path across renames
  (#59); cgit's own stats page doesn't either.
- Commit graph drawn as one inline SVG per row; no cgit-style `|\`/`|/` filler rows (#33).
- The log walk is left unsorted (no analogue of `commit-sort=date|topo`) — deliberate, to avoid
  O(repo size) per page (#33).
- No "Newer" pagination link — same choice as cgit's own pager UX (#18).
- axgit caps the *structured JSON* diffs at 1000 lines/file and 300 files; cgit has no diff size
  cap at all. `GET /rawdiff` and `GET /patch` (#37) match cgit here — no cap on either, since a
  truncated patch would be a corrupt one.
- Email addresses are never exposed in any response (only `email_hash`) — stricter than cgit's
  `noplainemail`.
- Archive format coverage (#54) matches cgit's `tar.gz`/`tar.bz2`/`tar.xz`/`tar.zst`/`zip` but
  deliberately **not** plain `tar` (little use as an HTTP download) or `tar.lz` (no maintained Rust
  lzip encoder — matching it would mean reintroducing the external-compressor-binary dependency
  #54 avoided for the other three).

**Replaced / no analogue planned**

- cgit's filter system (`about-filter`, `source-filter`, `commit-filter`, `email-filter`,
  `owner-filter`, exec + lua backends) → built-in client-side rendering: Shiki, react-markdown,
  DiceBear (#11). Consequence: **reStructuredText and man pages render as plain text**, and
  Gravatar is replaced by locally generated identicons.
- Dumb HTTP clone (`HEAD`/`info`/`objects` commands, `enable-http-clone`) → Smart HTTP upload-pack
  (#13). cgit has no Smart HTTP at all; the dumb protocol is not planned.
- `auth-filter` and per-repo auth — no analogue; authentication/authorization is out of scope
  entirely.
- Push — permanently excluded by the read-only invariant (cgit has no push either;
  `git-receive-pack` is 403).
- CGI/config-shape options with no analogue: `virtual-root`, `include`, macro expansion,
  `embedded`/`noheader`/`header`/`footer`/`head-include`, `css`/`js`,
  `scan-path`/`project-list`/`strict-export`/`scan-hidden-path`/`remove-suffix`/
  `section-from-path`, `mimetype.*`/`mimetype-file`, `enable-html-serving`, `case-sensitive-sort`,
  and the various `max-*` display caps. axgit's config surface is env vars plus each repo's
  `[cgit]`/`[axgit]` section, and repos always live one level under the root. `logo`/`logo-link`/
  `favicon` used to be grouped here too — moved back into scope as `AXGIT_LOGO`/`AXGIT_LOGO_LINK`/
  `AXGIT_FAVICON` (docs/DECISIONS.md #81) once the "no sibling static server to point a URL at"
  problem got a concrete answer (serve the file through axgit itself); `css`/`js`
  (arbitrary injection, not a single bounded asset) stay out.
- cgit's shipped `cgit.js` live relative-age refresh → static relative time is enough.
- cgit URL compatibility only covers the shapes in #35 — `tree/{path}?id=`, `plain/`, `atom/`, and
  `snapshot/` are deliberately not mapped.
- Single-child directory collapsing (cgit's `write_tree_link` renders `a / b / c` on one row) —
  judged low value, not planned.

**Not gaps (to avoid re-flagging)**

- Commit GPG signatures: cgit doesn't display or verify them either
  (`parsing.c::cgit_parse_commit` discards the `gpgsig` header). Snapshot `.asc` notes
  (`refs/notes/signatures/<fmt>`) are an unrelated feature, written only by a separate
  snapshot-signing setup that nothing in a normal push path runs, so this stays unplanned too.
- File-content search: cgit doesn't have it. axgit's `/search?type=content` is ahead here.
- Stats graphs: cgit's stats page is a plain numbers table, with no commits-vs-lines toggle either.
- Octopus-merge diffs: cgit shows no diff at all once a commit has 3+ parents.
