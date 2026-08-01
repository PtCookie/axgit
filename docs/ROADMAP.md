# Axgit Implementation Roadmap

Tracks the order and design of `api/` endpoint implementation across sessions. When finishing a
piece of work, update the "Done" section and replace "Next up" with the next target.

## Done

- `GET /api/v1/repos` — repository scanning (`api/src/repo/scan.rs`, `meta.rs`), TTL cache
  (`cache.rs`), reading `[axgit]`/`[cgit]` metadata. Remaining endpoints are 501 stubs;
  `git-receive-pack` is 403.
- `GET /api/v1/repos/{repo}`, `GET /api/v1/repos/{repo}/refs` — repository summary + refs. This
  established the common foundation used by everything after it: `repo/open.rs::open_named`
  (validates `{repo}` + opens bare, used by all subsequent per-repo endpoints),
  `repo/refs.rs::list_refs`, and `meta.rs`'s shared `git_time_to_zoned`/`format_rfc3339`. git2 work
  in handlers is wrapped in `spawn_blocking` (later endpoints follow the same pattern).
  ETag/Cache-Control were deferred (see the caching item below).
- `GET /api/v1/repos/{repo}/commits` — commit log (`repo/commits.rs::log`). Finalized design:
  - **Shared ref-resolution helper `repo/resolve.rs::resolve_commit`** (branch/tag/sha → Commit,
    any failure becomes `RefNotFound`). Reused by tree/blob/raw afterward. Bubbling a git2 error up
    with bare `?` turns into a 500, so any call driven by user input must `map_err`.
  - The cursor is inclusive and ignores `ref`. A malformed cursor is 400 (it's an opaque token).
    Exceeding `limit` is 400 (not clamped, per API.md).
  - The `path` filter compares git2 tree entry ids (not a `git log` exec). A merge is included only
    when it differs from all parents — an approximation of `git log` simplification. Paths that
    change rarely across a long history can make the walk slow: if that becomes a performance
    problem, consider falling back to `git log` exec.
  - `email_hash` = sha256(trim + lowercase). New test helper `tests/common::commit_history`
    (multi-commit history with fixed per-commit dates).
- `GET /api/v1/repos/{repo}/commits/{sha}`, `GET /.../commits/{sha}/diff?path=` — commit detail +
  structured diff (`repo/diff.rs`, new `handlers/commits.rs` — commit-related handlers, including
  `list_commits`, moved here; later endpoints follow the same per-resource handler file split).
  Finalized design:
  - Diff via git2 `diff_tree_to_tree` against the **first parent** (including for merges; root is
    diffed against an empty tree). Rename detection uses libgit2's default `find_similar`. An exec
    fallback is deferred until a bottleneck is confirmed.
  - Limits: 1000 rendered lines per file (cut at hunk boundaries), 300 files per diff response.
    Beyond that, `truncated: true`; `additions`/`deletions` are always the full totals. Binary
    files get `binary: true` + `hunks: []`.
  - `?path=` is a literal pathspec; a missing/unchanged path returns `files: []` (following log's
    precedent of an empty result; `PathNotFound` is reserved for tree/blob).
  - **Immutable Cache-Control rolled out early**: attached only when the request's `{sha}` matches
    the resolved full sha as a string (`handlers/commits.rs::sha_addressed_json`). A full
    ETag/response-cache rollout is still deferred.
  - New test helpers `tests/common::commit_all` (multi-file/delete/rename/binary commit setup),
    `get_json_with_headers`.

- `GET /api/v1/repos/{repo}/tree|blob|raw/{ref}/{path...}`, `GET /.../readme?ref=` — file browsing
  (`repo/tree.rs`, `blob.rs`, `readme.rs`, `handlers/files.rs`). Finalized design:
  - **`{ref}/{path...}` splitting via refs longest-match**
    (`repo/resolve.rs::resolve_ref_path`): the longest sequence of leading segments matching a
    branch/tag name is the ref (unambiguous per git ref rules), else the first segment is used as
    the ref (sha) fallback. `.`/`..`/empty segments in the path are 400.
  - Blob binary detection uses `Blob::is_binary()` plus non-UTF-8 counts as binary too.
    **1 MiB content limit** (`repo/blob.rs::BLOB_CONTENT_LIMIT`, also applied to readme). raw has
    no size limit; MIME is extension-based via mime_guess with a content fallback, plus `nosniff`.
  - readme searches case-insensitively in priority order; symlink/binary/over-limit candidates are
    skipped; 404 if none found.
  - `sha_addressed_json` moved to `handlers/mod.rs` to be shared (commits and files both use it).
    Tree entry size uses `odb().read_header()` (no content load).

- `GET /api/v1/repos/{repo}/archive/{ref}.{format}`, `GET /.../feed.atom` — archive streaming +
  Atom feed (`handlers/archive.rs`, `handlers/feed.rs`, DECISIONS #12). Finalized design:
  - archive uses `git archive` exec: after ref resolution, **only the full sha is passed to
    exec** (blocking injection), stdout is streamed via `tokio-util`'s `ReaderStream`, and a reaper
    task collects stderr + prevents zombies. `{ref}.{format}` parsing is suffix matching
    (`.tar.gz` first, then `.zip`, else 400). The filename/`--prefix` uses
    `{repo}-{safe_ref}`, replacing any character outside `[A-Za-z0-9._-]` with `-`.
    Only full-sha requests get immutable Cache-Control (existing precedent).
  - feed reuses `commits.rs::log` (HEAD, fixed at 20 entries); XML is generated by hand with an
    escaping helper (quick-xml is a dev-dependency for verification only). The base URL is
    reconstructed from `X-Forwarded-*`/`Host` headers; entry id is `urn:sha1:{sha}`. An empty
    repository returns 200 with 0 entries.
  - New test helper `tests/common::get_bytes_with_request_headers` (injects request headers).
    tar.gz is verified by actually extracting with `tar -xzf`; zip is byte-compared against
    `git archive`'s own output.

- `GET /{repo}.git/info/refs?service=git-upload-pack`, `POST /{repo}.git/git-upload-pack` —
  Smart HTTP upload-pack (`api/src/smart_http.rs`, DECISIONS #13). Finalized design:
  - Spawns `git upload-pack --stateless-rpc` directly (`--advertise-refs` for advertise, not
    http-backend CGI) — reuses archive's exec/ReaderStream/reaper pattern. Only the git dir
    resolved by `open_named` is passed to exec. The existing receive-pack 403 wiring is unchanged.
  - protocol v2 sanitizes the `Git-Protocol` header before passing it through as the `GIT_PROTOCOL`
    env var. The advertisement's pkt-line service header is prepended the same way for v2.
  - A gzip'd request body is fully buffered and decompressed with flate2 (compressed 8 MiB /
    decompressed 64 MiB limits). Writing stdin runs in a separate task (deadlock prevention).
  - Tests: oneshot protocol verification + round-tripping `git clone`/`--depth 1`/`fetch`/rejected
    push against a real listener (`tests/common::serve`) (must use the multi_thread flavor — the
    git CLI blocks the test thread).

- **moka response cache + ETag/Cache-Control rolled out everywhere** (`api/src/cache.rs`,
  `handlers/mod.rs`, DECISIONS #6). Resolved the debt deferred since summary/refs. Finalized
  design:
  - Validator `repo/meta.rs::Validator` = HEAD sha + agefile's **raw `SystemTime`**. Read via git2
    open then `head().target()` (not by parsing the HEAD file directly). Validator lookup also runs
    in spawn_blocking.
  - **The caching layer is a shared handler helper `handlers/mod.rs::cached_response`** (not tower
    middleware). The `compute` closure returns `(immutable, serialized body)`; on a miss, the
    validator and body come from the same open/snapshot. Errors are never cached and evict any
    existing entry. The old `sha_addressed_json` was removed as every handler moved to this
    helper.
  - Immutable entries store `validator: None` → on a hit, **the repository is never opened**
    (the only zero-git2 path). Everything else gets an ETag (validator-based, strong) +
    `no-cache` + 304.
  - Exceptions: the repo list keeps `ScanCache` + a body-sha256 ETag; raw is excluded from the
    cache (ETag only); archive can't be cached since it's streamed, so it gets a weak ETag (skips
    exec on a match); feed's cache key includes the base URL.
  - moka config: byte weigher + `AXGIT_CACHE_RESPONSE_MAX_BYTES` (32 MiB), 1 MiB per-entry body
    cap, TTL (`AXGIT_CACHE_RESPONSE_TTL` 300s) as an out-of-band-change safety net. `get_with`
    (request coalescing) isn't used — it doesn't fit the validator flow.
  - Tests: new `tests/cache_test.rs` (shares state via a cloned router — covers hit/invalidation/
    304/immutable entries). The cost of adding config fields is absorbed by centralizing
    `tests/common::test_config`/`router_for` in an earlier commit.
  - Follow-up: cursor- and full-sha-`ref`-addressed commit pages are effectively immutable too and
    could be promoted, but doing so would change the API.md contract ("sha appears in the URL
    path"), so it's deferred.

- `GET /api/v1/repos/{repo}/blame/{ref}/{path...}` — per-line-range attribution
  (`repo/blame.rs`, `handlers/files.rs::get_blame`). The last endpoint in v1 scope
  (DECISIONS #9, #14). Finalized design:
  - Adopted **git2 `Repository::blame_file`** (not exec) — the path never touches a command line,
    and ARCHITECTURE.md already assigns blame to git2. Adopted without a benchmark; if it proves
    slow on large histories, a `git blame --line-porcelain` exec fallback is a candidate for later
    review.
  - `{ref}/{path...}` splitting reuses the existing `repo/resolve.rs::resolve_ref_path`.
  - Binary/over-1-MiB is filtered via `blob::classify` (blob detection logic extracted into
    `blob_at`/`classify` and shared) into `ranges: []` + `lines: 0`. Same for empty files —
    filters out the 0-line hunk libgit2 returns.
  - Each hunk looks up its commit once via `final_commit_id()` and caches
    `summary`/`author`/`authored_at` (`HashMap<Oid, _>`) — the same commit appearing in multiple
    ranges isn't refetched. Added `Clone` to `CommitAuthor`, made
    `commits.rs::signature_info`/`time_rfc3339` public for reuse.
  - Caching follows tree/blob via `cached_response` (immutable when sha-addressed).
  - Follow-up: `git blame --follow` (rename tracking) is not adopted — revisit alongside the exec
    fallback if it's ever needed.

- **OpenAPI spec + Swagger UI + web type generation** (`api/src/openapi.rs`, `docs/openapi.json`,
  `web/src/lib/api/types.ts`, DECISIONS #15). Made the contract machine-readable before the web
  implementation, eliminating the "manually define types.ts" debt. Finalized design:
  - utoipa 5 + utoipa-swagger-ui 9 (`vendored`). The spec is served at `/api/v1/openapi.json`, the
    UI at `/swagger-ui`. `utoipa-axum` auto-collection is **not used** — because of the catch-all
    route, paths are written by hand in `openapi.rs`.
  - `docs/openapi.json` is committed, and `tests/openapi_test.rs` verifies (1) snapshot equality
    (2) routed operations == spec operations (16) (3) a smoke test of both routes. Regenerate with
    `AXGIT_UPDATE_OPENAPI=1 cargo test --test openapi_test`.
  - `&'static str`/`char` fields promoted to enums (`DiffStatus`/`EntryKind`/`LineOrigin`/
    `ReadmeFormat`); always-serialized `Option`s get `#[schema(required = true)]`. JSON output is
    unchanged — proven by the existing 129 integration tests still passing as-is. Error bodies are
    now typed as `ErrorResponse`/`ErrorBody` structs.
  - web generates `types.ts` via `pnpm gen:types` (openapi-typescript + prettier). Excluded from
    eslint.
  - Corrected 2 instances of API.md drift: a stale `501 not_implemented` row, and a mismatched
    `charset=utf-8`.

- **Moved the pnpm workspace to the repo root** (DECISIONS #7). Because `web/` had been scaffolded
  as its own standalone pnpm project, the `pnpm install` (workspace root) and
  `pnpm --filter web ...` documented in CLAUDE.md/README.md never actually worked. Added a root
  `package.json`/`pnpm-workspace.yaml`, and renamed `web/package.json`'s `name` to `"web"` so the
  filter matches. Also moved `web/pnpm-workspace.yaml`'s `allowBuilds` to the root (pnpm only reads
  it there). `lefthook.yml` already used the `root: "web/"` style, so it needed no changes.

- **web app shell + `/` repository list page + API client** (DECISIONS #16). Removed the leftover
  Astro template scaffolding (`Welcome.astro`, demo assets) and stood up the actual first screen.
  Finalized conventions:
  - `web/src/lib/api/schemas.ts` — re-exports convenient type aliases from the generated
    `types.ts`. `client.ts` provides `apiFetch<T>`/`ApiError` (parses the error envelope,
    normalizes non-JSON/network failures too), and thin per-resource wrappers (`repos.ts`) sit on
    top. Later pages should follow this pattern.
  - Component tests use **vitest browser mode** (`@vitest/browser-playwright` provider +
    `vitest-browser-react`), living in `web/tests/` (`vitest.config.ts`'s `include` only matches
    that path). e2e is Playwright, in `web/e2e/`. Both mock the API via `page.route`/`vi.mock` so
    they run without a backend.
  - Section grouping (`RepoList.tsx`): `section: null` always groups as "Other," last. Other
    groups preserve their first-appearance order in the response (= repo name sort order).
  - `web/src/lib/format/time.ts` (relative/absolute time) — planned for reuse in the commit
    log/blame pages.
  - Per-repository subpaths (`/{repo}/...`) weren't built in this commit — they land in the next
    commit, together with the SPA fallback wiring.

- **SPA fallback wiring + repository summary/refs pages** (DECISIONS #16). Made `/{repo}/...`
  actually routable. Finalized design:
  - api side: `api/src/routes.rs` — attached `.fallback(api_not_found)` to the `/api/v1` router to
    return `NotFound` (`error.rs`, `not_found` code), and switched static file serving to
    `ServeDir::new(static_dir).fallback(ServeFile::new(index.html))` so any path without a matching
    file gets the SPA shell. Because axum's `nest`ed routers inherit the outer
    `fallback_service`, the two fallbacks had to be wired separately — otherwise
    `/api/v1/bogus` would get the HTML shell too, without the API fallback in place.
    `docs/API.md`'s error code table and `docs/openapi.json` were updated in the same commit
    (ErrorBody description text).
  - `astro dev` doesn't have this fallback, so a vite middleware (`spaFallback`) was added to
    `astro.config.mjs` so `/{repo}/...` navigation rewrites to `/` in dev/e2e too (the static build
    itself is untouched).
  - web side: **the client router parses `location.pathname` once, with no History API**
    (`lib/router.ts::parseRoute`) — premised on every navigation being a full page load, so the
    route never changes mid-mount. `components/App.tsx` mounts with `client:only="react"` (since
    the static build only has one HTML file for `/`, `client:load`'s build-time render would
    mismatch any other route). Only four routes exist so far —
    `repos`/`repo` (summary)/`refs`/`not-found` — log/tree/blob/commit/blame will be added to
    `parseRoute` in the next commit.
  - `RepoSummary.tsx`/`RefsView.tsx` follow the same loading/error/data state pattern as
    `RepoList.tsx` (component tests added with the same structure as `RepoList.test.tsx` too).
    `RepoNav.tsx` is stateless tab navigation (`<a>` — plain anchors instead of shadcn `Tabs`, for
    the same full-page-load reasons as above).
  - New `web/src/lib/api/path.ts::encodeSegment` (escapes a repo name as a URL segment),
    added `getRepo`/`getRefs` to `repos.ts`.
  - e2e (`web/e2e/repo.spec.ts`) covers summary → refs tab navigation and the not-found display for
    an unsupported subpath.

## Next up: remaining per-repository pages (log, tree, blob, commit, blame)

### Context

`/{repo}/` (summary) and `/{repo}/refs` are in place, and the SPA fallback + client router
(`lib/router.ts`) conventions are established. Next: attach the remaining per-repository subpages
using the same conventions.

### Things to review before starting

- Routes: `/{repo}/log`, `/{repo}/tree/[...path]`, `/{repo}/blob/[...path]`,
  `/{repo}/commit/{sha}`, `/{repo}/blame/[...path]`. ref selection is unified via the `?ref=` URL
  query. Add each route to `lib/router.ts::parseRoute`/`Route` and match them in `App.tsx`'s
  switch.
- Unlike the API, where tree/blob/blame all use a single `{ref}/{path...}` catch-all, the client
  route has a variable number of `path` segments, so `parseRoute` needs to decide the `ref`/`path`
  boundary itself — decide whether to reimplement the API's branch/tag longest-match on the
  client, or keep `?ref=` as-is and treat everything else as `path` (the latter seems to fit the
  API contract better; confirm again when starting). Also add each route to `RepoNav.tsx`'s `TABS`
  array.
- Everything else follows existing conventions: generated types get an alias added in
  `schemas.ts`, the API client reuses `client.ts`'s `apiFetch`, shadcn components go in
  `web/src/components/ui/`, tests are vitest browser mode (`web/tests/`) + Playwright
  (`web/e2e/`).
