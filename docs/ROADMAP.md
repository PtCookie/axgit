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

- **Serve prerendered page shells per route shape** (DECISIONS #17, refining #16). Astro pages
  became the routing layer again instead of a client-side-routed SPA. Finalized design:
  - web side: `src/pages/[repo]/index.astro` and `[repo]/refs.astro` prerender once under a
    reserved `getStaticPaths` param (`__repo__`, `web/src/lib/shell.ts::REPO_SHELL_PARAM`),
    producing `dist/__repo__/index.html` and `dist/__repo__/refs/index.html` alongside
    `dist/index.html` and the new `dist/404.html` (`pages/404.astro`, real 404 content instead of
    the deleted `NotFound.tsx`). Deleted `components/App.tsx`, `lib/router.ts`,
    `components/repo/RepoNav.tsx`, `components/NotFound.tsx`.
  - Page chrome (heading, tab nav, `<title>`) is now static HTML: `layouts/RepoLayout.astro` wraps
    `Layout.astro` and renders the new `components/repo/RepoNav.astro` (a static tab bar, `<a>`
    hrefs marked `data-repo-href`) plus one `is:inline` script that fills in the repository name,
    tab hrefs, and `document.title` from `location.pathname`'s first segment before first paint. A
    bundled `<script>` was rejected — Astro 5+ defers those (`type="module"`), which would flash an
    empty heading.
  - Data islands (`RepoList`, `RepoSummary`, `RefsView`) stay `client:only="react"` (not
    `client:load` — see DECISIONS #17) but now render into a static `slot="fallback"` skeleton, each
    extracted as a named export (`RepoListSkeleton` etc.) shared with the component's own
    `loading` state. `RepoSummary`/`RefsView`'s `repo` prop became optional, defaulting to
    `lib/repo-param.ts::repoFromPathname(window.location.pathname)` — existing component tests are
    unaffected since they pass `repo` explicitly.
  - api side: `api/src/shell.rs` (new module) maps a request path's *shape* to its shell file —
    `shell_for` is a pure function with its own unit tests; `serve_shell` reads the file per request
    (not slurped at startup, so `astro build` output is picked up without an api restart).
    `api/src/routes.rs`'s static fallback changed from `ServeDir::fallback(ServeFile(index.html))`
    to `ServeDir::fallback(get(shell::serve_shell))`. Unmatched paths now answer a real `404` instead
    of the old blanket `200`.
  - dev parity: `astro.config.mjs`'s `spaFallback` middleware became `shellFallback`, driven by the
    same `web/src/lib/shell.ts::shellFor` the pages/api rely on. Also fixed a latent bug — the old
    middleware dropped the query string on rewrite, which would have broken a future `?ref=` link.
  - `api/tests/spa_fallback_test.rs` renamed to `static_shell_test.rs` and rewritten around the
    four shells; added coverage for Smart HTTP and Swagger UI taking precedence over the fallback
    (nothing previously exercised that with a static dir configured).
    `web/tests/lib/router.test.ts` replaced by `web/tests/lib/shell.test.ts`. `web/e2e/*.spec.ts`
    needed no changes (they assert visible content and URLs, not HTTP status).
  - No API contract change — `docs/API.md`/`docs/openapi.json`/`web/src/lib/api/types.ts` untouched.

- **`/{repo}/log` + `/{repo}/commit/{sha}` pages**. The first vertical slice through the
  commit-centric endpoints (log list → commit detail), and the pair the roadmap had flagged as
  needing a decision before starting. Finalized design:
  - **Ref selection stays `?ref=` only** — path segments are never treated as a ref. This was the
    open question from the previous entry; reimplementing the API's branch/tag longest-match
    client-side was rejected as unnecessary complexity, and `?ref=`-only matches
    ARCHITECTURE.md's existing "ref selection is unified via the `?ref=` URL query" line.
    `shellFor`/`shell_for` therefore still don't need to know about refs at all.
  - Routing (DECISIONS #17's three-places-at-once cost): `web/src/pages/[repo]/log.astro` and
    `[repo]/commit/[...sha].astro` (the rest param is built with `sha: undefined`, producing one
    `commit/index.html` — the real sha is read from `location` at runtime, same as the repo name
    elsewhere). `shellFor`/`shell_for` each gained two shapes; `shell_for`'s match grew a 4th
    lookahead segment so `/{repo}/commit/{sha}/extra` still 404s. `RepoNav.astro` gained a "Log"
    tab; commit detail isn't its own tab and maps back to "Log" for the active-tab highlight
    (`RepoLayout.astro`).
  - **Pagination is a plain anchor** (`Older →`, carries `cursor`/`ref`/`path`), consistent with
    #17's "no client-side router" premise — there's no "Newer" link, the browser back button
    covers it (cgit's own UX).
  - New `web/src/components/repo/CommitLog.tsx`/`CommitView.tsx` follow the established
    `RefsView`/`RepoSummary` state-machine pattern (`loading | error | data`, `cancelled` flag,
    optional `repo?`/`sha?` props defaulting to `location`-derived values for `client:only`).
    `CommitView` fetches detail + diff in parallel (`Promise.all`). Diff rendering has no size
    virtualization — acceptable given the API's own 1000-line/300-file caps.
  - **Avatars**: `web/src/components/repo/AuthorAvatar.tsx` adopts DECISIONS #11's plan —
    `@dicebear/collection`'s `identicon` style seeded from `email_hash`, rendered as a
    `toDataUri()` `<img>` (no external request). Pulled in `@dicebear/core`/`@dicebear/collection`
    as new deps; pinned `@dicebear/core` to `^9.4.3` (not the newest v10) since `@dicebear/collection`
    only declares a `^9.0.0` peer range — the two majors don't interop (v10 dropped the `escape`
    export several style packages still import).
  - **Commit message linkification**: `web/src/lib/format/linkify.tsx` — regex-based (URL / bare
    7–40-char hex sha), returns React nodes rather than using `dangerouslySetInnerHTML` since
    commit messages are untrusted repo content.
  - `RepoList.tsx` repository names became links to `/{repo}` (previously dead-ended — no path led
    from the list into a repository at all).
  - No API contract change — `docs/API.md`/`docs/openapi.json`/`web/src/lib/api/types.ts`
    untouched; `schemas.ts` gained re-exported aliases for the commit/diff schema types.

- **`/{repo}/tree` + `/{repo}/blob` pages, with Shiki syntax highlighting** (DECISIONS.md #19).
  Closed the biggest remaining gap versus cgit — there was previously no way to browse into a
  repository's files at all. Finalized design:
  - **`shellFor`/`shell_for` generalized to segment-slice matching** (`web/src/lib/shell.ts`,
    `api/src/shell.rs`) — tree/blob paths have unbounded depth, which the old fixed-arity tuple
    match (#17) couldn't express. `/{repo}/tree` (no path) is the root tree and is valid;
    `/{repo}/blob` (no path) 404s, since there's nothing to display.
  - **Fixed a latent dev-only bug**: `web/astro.config.mjs`'s `shellFallback` used an
    `extname(pathname) === ""` heuristic to detect app routes, which wrongly skipped the rewrite
    for any dotted path (a blob path like `/{repo}/blob/src/main.rs`) — 404ing in `astro dev`/
    Playwright only. Replaced with an actual `public/`-file existence check.
  - Routing: `web/src/pages/[repo]/tree/[...path].astro`, `[repo]/blob/[...path].astro`
    (`getStaticPaths` with `path: undefined`, same rest-param pattern as `commit/[...sha].astro`).
    `RepoNav.astro` gained a "Tree" tab; blob maps back to "Tree" for the active-tab highlight
    (`RepoLayout.astro`), same as commit → log.
  - New `web/src/components/repo/PathBreadcrumbs.tsx` (shared by tree/blob, exports `treeHref`),
    `TreeView.tsx` (entries table; `tree`/`blob`/`symlink` link, `commit` gitlinks don't),
    `BlobView.tsx` (binary/too-large/symlink branches, `Raw`/`History` links), `CodeBlock.tsx`
    (line-numbered viewer, renders plain text immediately then swaps in Shiki tokens — no loading
    flash). `lib/repo-param.ts` gained `filePathFromPathname`/`paramFromSearch` (the latter
    factored out of `CommitLog.tsx`, which now reuses it too). `lib/api/repos.ts` gained
    `getTree`/`getBlob`/`rawUrl`; `ref` continues to be `?ref=`-only (#18) — when absent, the
    literal `HEAD` is sent as the API path's `{ref}` segment rather than resolving a default
    client-side.
  - **Shiki**: `shiki/core` + `shiki/engine/javascript` (JS RegExp engine, not Oniguruma/WASM —
    avoids a ~500 KiB wasm asset). `lib/format/highlight.ts` lazy-loads ~20 common languages by
    extension; anything else, or content over 512 KiB/5000 lines, renders as plain text. Both
    `github-light`/`github-dark` themes are tokenized together and each token's Shiki-provided
    `htmlStyle` (`color` + `--shiki-dark`) is used directly as a React inline style — `global.css`
    adds the matching `.dark .shiki-code span` override (inert until a dark-mode toggle exists).
  - No API contract change — `docs/API.md`/`docs/openapi.json` untouched; `schemas.ts` gained
    `TreeListing`/`TreeEntryInfo`/`EntryKind`/`BlobInfo` aliases.

- **`/{repo}/blame/[...path]` page** (DECISIONS.md #20). The last v1-scope web page — closed the
  final gap versus cgit. Finalized design:
  - `BlameView` fetches `getBlame`/`getBlob` in parallel (the blame response has no file content)
    and renders `blob.content` with the ranges laid over it.
  - `CodeBlock.tsx` gained an optional `gutter` prop (`(GutterCell | null)[]`, `rowSpan`-merged
    per range) instead of a separate blame code viewer; omitted by every existing caller, so
    tree/blob rendering is unaffected.
  - Gutter is compact cgit-style: short sha link + relative time + author name, summary as a
    tooltip only.
  - Entry is blob-only — a "Blame" link on `BlobView`, no dedicated nav tab (`RepoLayout.astro`
    treats `blame` like `blob` for the active-tab highlight). Routing/shell wiring
    (`web/src/pages/[repo]/blame/[...path].astro`, `shellFor`/`shell_for`) follows blob's pattern.
  - `treeHref` moved out of `PathBreadcrumbs.tsx` into new `web/src/lib/repo-href.ts`, joined by
    `blobHref`/`blameHref` — three components now need repo-page hrefs.
  - No API contract change — `docs/API.md`/`docs/openapi.json` untouched; `schemas.ts` gained
    `BlameInfo`/`BlameRange`, `lib/api/repos.ts` gained `getBlame`.

- **README rendering + archive/feed links** (DECISIONS.md #21). Closed the last "implemented but
  unused by the web app" gap. Finalized design:
  - `web/src/components/repo/ReadmeView.tsx` — new island, mounted on `/{repo}` below
    `RepoSummary` (`pages/[repo]/index.astro`). react-markdown + remark-gfm + rehype-sanitize (no
    `rehype-raw`) for `format: "markdown"`; `rst`/`plain` render as `<pre>` (#11's plan, as-is).
    A 404 (`path_not_found`/`ref_not_found`) renders nothing, not an error — kept as its own fetch/
    island specifically so that doesn't touch `RepoSummary`'s error state.
  - `web/src/lib/markdown-url.ts` (new, unit-tested) rewrites README-relative links/images to
    `treeHref`/`blobHref`/`rawUrl` — otherwise every relative reference 404s against the page URL.
  - `lib/format/highlight.ts` split its tokenizing tail into a private `tokenize`, adding
    `highlightFence`/`languageForFence` alongside the existing `highlightCode`/`languageForPath` —
    README code fences get Shiki highlighting too, via `ReadmeView`'s `pre` override + local
    `MarkdownFence`. Hit a real bug here (see DECISIONS.md #21): react-markdown substitutes a
    `code` override as the element's `type` itself, not the string `"code"`, so the fence-detection
    check must compare identity against the component reference; the fix is now covered by a
    component test that asserts on the actual highlighted DOM, not just fence text.
  - `RepoSummary.tsx` gained two `dl` rows — `archiveUrl`/`feedUrl` (new, link-only, same pattern
    as `rawUrl`) for HEAD-only tar.gz/zip download + an Atom feed link, hidden for an empty
    repository alongside the existing "No commits yet." branch.
  - No new page/route, no `shellFor`/`shell_for` change. No API contract change — `schemas.ts`
    gained `ReadmeInfo`/`ReadmeFormat` aliases.

- **Dockerfile / single-container build** (DECISIONS.md #22). Closed the last gap between the
  current state and actually replacing cgit in a live git-compose deployment. Finalized design:
  - 3-stage `Dockerfile`: `node:24.11-alpine3.22` builds `web/dist` (`pnpm install
    --frozen-lockfile` → `pnpm --filter web build`), `rust:1.97-alpine3.22` builds the `axgit`
    release binary (`musl-dev` added so `libgit2-sys` builds vendored libgit2 with `cc` — alpine
    has no system libgit2 for pkg-config to find), and `alpine:3.22` is the runtime (`git` +
    `ca-certificates` only, no `tzdata` — jiff never touches the system tzdb). All three base
    images are pinned to a minor version via top-of-file `ARG`s, not floating tags.
  - Because `git2` is `default-features = false` (no ssh/https transports) and
    `utoipa-swagger-ui`'s `vendored` feature is on, the whole build is network-free past dependency
    fetch — no openssl/libssh2 wrangling, no swagger-ui zip download. The musl binary ends up
    fully static (confirmed via `ldd`), so the runtime stage needs no libgit2/zlib shared libs.
  - **`/etc/gitconfig` gets `[safe] directory = *`** (plus a dedicated non-root uid 10001 user) —
    `/srv/git` is a read-only bind mount owned by the git-server container's uid, which trips both
    `git` exec's and libgit2's ownership check; one gitconfig file covers both since both read the
    system config. Verified end-to-end (clone/`--depth 1`/fetch/push-403/archive/every web route)
    against `fixtures/repos` mounted `:ro` as a foreign uid.
  - BuildKit cache mounts for pnpm's store and cargo's registry + `api/target`; the release binary
    is `cp`'d out to a plain path in the same `RUN` (cache mount contents don't persist into the
    image layer).
  - New `.dockerignore` (`.git`, `node_modules`, `api/target`, `web/dist`, test artifacts,
    `fixtures/repos`) — `api/target` alone was 9G, which would otherwise bloat the build context.
  - `docs/compose.example.yaml` (new) — an illustrative `git-web`-replacement service definition;
    actual git-compose.git changes are tracked in that separate repo. `README.md` gained a
    Deployment section (build/run examples, the env var table from `config.rs`, an arm64/amd64
    `--platform` note).
  - No API contract change — `docs/API.md`/`docs/openapi.json`/`web/src/lib/api/types.ts`
    untouched. No `api/src` changes were needed at all; `config.rs`'s existing env-driven `Config`
    already covered every setting the image needs.

- **3-way theme selector, System / Light / Dark** (DECISIONS.md #23). Activated the `.dark` token
  block `global.css` has carried since #19 without anything ever applying the class. Finalized
  design:
  - `Layout.astro`'s `<head>` gained an `is:inline` script that reads `localStorage["axgit:theme"]`
    (try/catch — storage throws outright when disabled) falling back to `prefers-color-scheme`, and
    sets both the preference (`<html data-theme>`) and the resolved value (the `dark` class) before
    the document has a body — the same "flash of empty content" lesson #17 recorded for
    `RepoLayout.astro`'s fill-in script.
  - The control is `components/ThemeToggle.tsx`, a `client:only="react"` island (`RepoList`'s
    pattern) using shadcn's `dropdown-menu` (Base UI `Menu`, already a dependency via
    `ui/button.tsx`) with a `RadioGroup`/`RadioItem` for the three options. Its trigger renders all
    three System/Light/Dark Phosphor icons unconditionally; a `global.css` rule reveals only the one
    matching `<html data-theme>`, so the prerendered static fallback already shows the right icon
    before the island hydrates. `components.json`'s `iconLibrary: "phosphor"` was declared but
    unused until now.
  - `global.css`: `color-scheme` on `:root`/`.dark`, `@custom-variant dark` widened to
    `&:is(.dark, .dark *)` (the class lands on `<html>` itself, which shadcn's descendant-only form
    excludes), and the icon-visibility rule above. `.dark --primary` was raised from
    `oklch(0.432 …)` to `oklch(0.72 0.13 166)` — shadcn's generated value was *darker* than light
    mode's, invisible until `.dark` went live, and put every `text-primary` link and the active-tab
    underline under ~3:1 contrast.
  - Only one place in the app needed a dark variant added — `CommitView.tsx`'s
    `bg-green-500/10`/`bg-red-500/10` diff-line backgrounds; everything else was already on
    semantic tokens. Shiki's dual-theme output (#19) needed no change and works as designed; this
    is the first time `.dark .shiki-code span` has ever been exercised.
  - `web/e2e/theme.spec.ts` (new) covers default-from-OS resolution, explicit override, reload
    persistence, live OS-change tracking while set to `system`, and that the served shell bakes in
    no theme of its own. Untestable at the vitest-browser-mode layer by construction (no Astro
    layout is rendered there), same as `RepoLayout.astro`'s script.
  - No new page/route, so no `shellFor`/`shell_for` change. No API contract change.
- **Client-side routing via Astro's `<ClientRouter />`** (docs/DECISIONS.md #24). Same-origin link
  clicks now swap `<body>` in place instead of a full page load, with a short fade on `<main>`.
  - `<header>` is `transition:persist`ed (keeps the `ThemeToggle` island alive across navigations);
    `<main>` and every data island inside it are not, so islands always remount fresh against the
    new URL rather than receiving props on a live instance — required since they all read
    `location` only at mount (`lib/repo-param.ts`).
  - Astro's swap replaces `<html>`'s whole attribute set, which resets the theme (#23) on every
    navigation unless reapplied — `Layout.astro`'s theme script became a named function invoked
    again on `astro:after-swap`. Same event carries `RepoLayout.astro`'s shell fill-in logic,
    relocated to `Layout.astro` as `window.__axgit.fillRepoShell` (present on every page, so the
    listener is attached before the first navigation into a repository page) and keyed off
    `data-title-suffix` on the heading rather than `document.currentScript`, which is `null` when
    Astro re-runs a script post-swap.
  - `data-astro-rerun` was rejected for both of the above: Astro's re-run scripts execute after the
    transition's DOM update finishes (after paint), while `astro:after-swap` fires before it —
    confirmed by reading Astro 7.1.6's router source directly.
  - `prefetch.prefetchAll: false` set in `astro.config.mjs`, overriding the `true` default
    `<ClientRouter />` enables — every `/{repo}/blob/*`-shaped URL maps to one byte-identical,
    `Cache-Control: no-cache` shell (#17), so prefetching it on hover has no upside.
  - Dev middleware (`astro.config.mjs::shellFallback`) widened to also match the router's own
    fetches (`Sec-Fetch-Dest: empty`, no `Accept: text/html`), or `astro dev`/Playwright would 404
    every client-side navigation and silently fall back to full reloads.
  - `global.css` disables the root view-transition animation so the persisted header/tab bar swap
    instantly instead of cross-fading under `<main>`'s own fade.
  - `web/e2e/theme.spec.ts` gained a regression test for theme survival across a client-side
    navigation; `web/e2e/repo.spec.ts` gained a marker-survival assertion that's the only signal
    a test is exercising client-side routing rather than a downgraded full reload. No new
    route/page, no API contract change.

- **Repository list filter, client-side (`?q=`)** (DECISIONS.md #25). The first cut of #9's
  deferred repository search. Finalized design:
  - `web/src/lib/repo-filter.ts::filterRepos` — AND across whitespace-separated terms, each matched
    case-insensitively as a substring of `name`/`description`/`owner`/`section` (`null` fields
    skipped). Filtering happens before `RepoList.tsx`'s existing `groupBySection`, so an
    unmatched section just doesn't render. Zero API change: `GET /api/v1/repos` already returns
    every field searched.
  - `RepoList.tsx` mirrors the typed query into `?q=` via `history.replaceState` (not `pushState` —
    one entry per keystroke would break the back button) and seeds it back out via the existing
    `lib/repo-param.ts::paramFromSearch`, so a deep link/reload lands already filtered. Doesn't
    interact with `<ClientRouter />` (#24), which only reads `location` on a link click.
  - New `web/src/components/ui/input.tsx` (`pnpm exec shadcn add input`, vendored like every other
    `ui/` file) — the search box. `RepoListSkeleton` grew a matching disabled input so the
    prerendered fallback and loading state don't shift layout once data arrives.
  - No new page/route, so no `shellFor`/`shell_for` change. No API contract change —
    `docs/API.md`/`docs/openapi.json`/`web/src/lib/api/types.ts` untouched, `api/src` untouched.

- **`GET /api/v1/repos/{repo}/search` — content/path/commit-message search** (DECISIONS.md #26).
  The second half of #9's deferred "repository search" (#25 was the client-side list filter) —
  searching *inside* a repository. Finalized design:
  - git2 in-process scan (`repo/search.rs`), not a `git grep` exec or a persistent index — an
    index would be this app's first piece of mutable, persistent state in an otherwise stateless,
    read-only container. `type=content|path|message` is one endpoint, not three; all three share
    the "resolve a ref, walk something, cap the results" shape and response envelope.
  - **Two independent budgets**: `limit` (1–100, default 50, same rule as the commit log) caps
    *results*; a separate scan budget caps *work done regardless of matches* — 20,000 tree entries
    walked, 32 MiB of blob content actually read (checked via `Odb::read_header` before a blob is
    loaded), 10,000 commits walked for `message`. Either sets `truncated: true`. Content search
    reuses `repo/blob.rs::classify` (binary/1 MiB cap) so a search never surfaces something the
    blob view itself would refuse to render.
  - `q` is a fixed string (not regex), case-insensitive, 1–200 characters.
  - Caching/immutability and the empty-repository (unborn HEAD) carve-out both follow existing
    precedent exactly (commit-detail's full-sha immutability rule; the commit log's "omit `ref` on
    an empty repo → 200 with empty results, explicit `ref` → 404" rule).
  - `handlers/commits.rs`'s `parse_limit`/`DEFAULT_LIMIT`/`MAX_LIMIT` moved to `handlers/mod.rs`
    (`pub(crate)`) as the second caller; `repo/commits.rs::commit_info` promoted to `pub(crate)` so
    `type=message` results reuse `CommitInfo` rather than a parallel struct.
  - `docs/API.md`/`docs/openapi.json`/`web/src/lib/api/types.ts` all updated (new endpoint);
    `schemas.ts` gained `SearchResults`/`SearchKind`/`FileMatch`/`LineMatch` aliases.

- **`/{repo}/search` page** (DECISIONS.md #27). The web UI for the endpoint above — closes out
  repository search (#9/#25/#26). Finalized design:
  - Route `web/src/pages/[repo]/search.astro` follows `log.astro`'s shape (no path segments, all
    state in `?q=&type=&ref=`) — one new two-segment case in `shellFor`/`shell_for`.
    `RepoNav.astro` gained a "Search" tab; `RepoLayout.astro`'s `Active`/`TITLE_SUFFIXES`/
    `NAV_ACTIVE` grew a `search` case (its own tab, not a drill-down).
  - **Plain `<form method="get">`, no controlled inputs.** Astro's `<ClientRouter />` (#24)
    intercepts same-origin GET form submits the same way it does link clicks (confirmed by reading
    7.1.6's source directly), so no `onSubmit` handler was needed and a no-JS fallback still works.
  - `SearchView.tsx` (+ `SearchViewSkeleton`) follows the `CommitLog`/`TreeView` state-machine
    pattern; `content`/`path` results link into the blob view via new `repo-href.ts::blobLineHref`
    (`#L{n}`, reusing `CodeBlock.tsx`'s existing anchor scheme — no new mechanism needed);
    `message` results reuse `CommitLog`'s row shape. A `truncated: true` response renders a visible
    banner.
  - `lib/api/repos.ts` gained `searchRepo`/`SearchParams`; `repo-href.ts` gained
    `blobLineHref`/`searchHref`; `schemas.ts` gained the search type aliases. No API contract
    change.

- **`GET /api/v1/repos/{repo}/stats` — commit-activity statistics** (DECISIONS.md #28). The other
  half of #9's v1 exclusions (repository search, #25/#26/#27, was the first) — cgit's `stats` page.
  Finalized design:
  - git2 in-process revwalk (`repo/stats.rs`), same reasoning as search: no persistent index, no
    `git log` exec — bounded by `MAX_SCANNED_COMMITS = 20_000`.
  - **The window is anchored on the resolved commit's authordate, not the request time** —
    makes a full-sha `ref` response deterministic, so it gets the same immutable-caching
    treatment as commit detail/diff/blame via `handlers/mod.rs::cached_response`.
  - Always **12 buckets** regardless of `period` (`week`/`month`/`quarter`/`year`, default
    `month`); boundaries computed with jiff's calendar-aware `Span` arithmetic (month/quarter/year
    lengths vary), in UTC. Out-of-window commits are dropped; commits authored past the window's
    end clamp into the last bucket rather than being dropped (no early-exit heuristic — revwalk
    order isn't strictly chronological across merges).
  - `authors[].buckets` is a parallel array to the top-level `buckets`. `limit` (reusing
    `handlers/mod.rs::parse_limit`, now a third caller) caps only the `authors` rows, never bucket
    totals; `truncated` covers both the scan budget and the `limit` cut, matching search's
    precedent.
  - `docs/API.md`/`docs/openapi.json`/`web/src/lib/api/types.ts` all updated (new endpoint);
    `schemas.ts` still needs its stats aliases (deferred to the page commit below).

- **`/{repo}/stats` page** (DECISIONS.md #29). The web half of #28 — closes out #9's v1 exclusion
  list entirely (HTTP push remains permanently excluded by the read-only invariant, not deferred).
  Finalized design:
  - Route `web/src/pages/[repo]/stats.astro` follows `search.astro`'s shape (no path segments, all
    state in `?period=&ref=`) — one new two-segment case in `shellFor`/`shell_for`. `RepoNav.astro`
    gained a "Stats" tab; `RepoLayout.astro`'s `Active`/`TITLE_SUFFIXES`/`NAV_ACTIVE` grew a `stats`
    case. `theme.spec.ts`/`repo.spec.ts`'s 404-shell test fixture moved from `/git-compose/stats`
    (no longer unmatched) to `/git-compose/blob` (still structurally unmatched with no path).
  - **Recharts, installed via `pnpm exec shadcn add chart`** (not a separate `pnpm add`) —
    confirmed the `base-luma` style's `chart` registry item declares `recharts` as its own
    dependency and pulls in `registryDependencies: card`, so `ui/chart.tsx` + `ui/card.tsx` both
    vendored in one step. The vendored file's one lint error was `eslint --fix`ed; its remaining
    warnings are left as shadcn generated them (CLAUDE.md's "generated files may be modified"
    policy — fix what's broken, not the whole file's style).
  - **`--chart-1` was genuinely broken and got fixed via the `dataviz` skill's validator**, not by
    eye — light mode measured 1.44:1 contrast (FAIL, near-invisible) and dark mode's lightness sat
    above the dark band (FAIL, would glow). Fixed by reusing `--primary`'s hue at two different
    validator-passing lightness steps (light reuses `--primary` itself; dark needed a new L 0.6
    step, since `--primary`'s own dark value was tuned for text contrast, not a chart mark's
    dark-band requirement). `--chart-2..5` remain unvalidated — untouched until a second series
    exists.
  - **`minPointSize={2}` on the `<Bar>`** — a real bug the skill's mandatory "render it and look at
    it" step caught: Recharts omits the bar element entirely for a zero-commit bucket, leaving that
    month with no hover/tooltip hit target. Confirmed via rendered-SVG inspection (11 bars for 12
    buckets before the fix, 12 after).
  - Period switcher is four plain links (`statsHref`), not a form — a fixed 4-way pick needs no
    free-text input, and `<ClientRouter />` (#24) already intercepts the clicks.
  - The author table (`authors[].buckets`, parallel to the response's `buckets`) doubles as the
    chart's required "table view" — a `TableFooter` "Total" row sums each bucket column, so every
    number the chart plots is also reachable without it.
  - `web/src/lib/api/repos.ts` gained `getStats`/`StatsParams`; `repo-href.ts` gained `statsHref`;
    `schemas.ts` gained the stats aliases. No API contract change — web-only commit.

- **Lazy-load Recharts, react-markdown, and the theme menu** (DECISIONS.md #30). Picked up the
  "web build's largest JS chunk" candidate below — the premise there was wrong once measured (the
  actual 500 kB-plus chunk is Shiki's already-lazy `cpp` grammar, not Recharts). The real fix was
  splitting three *eagerly*-loaded chunks behind `React.lazy`/`Suspense`:
  `ThemeToggle`/`ThemeMenu` (137 KB → ~2 KB eager, loaded on every page), `StatsView`/`StatsChart`
  (345 KB → ~5 KB, pre-warmed alongside the `/stats` fetch), `ReadmeView`/`ReadmeMarkdown`
  (145 KB → ~2 KB, deliberately *not* pre-warmed so a README-less repo downloads none of it).
  `phosphor-icons`' barrel import was checked and confirmed to already tree-shake correctly — left
  alone. No API contract change, no new route.

- `/{repo}` summary as a two-column layout (DECISIONS.md #31). The README becomes the wide main
  column; the repository description + metadata move into a narrow right sidebar (GitHub's "About"
  shape). The grid lives in `pages/[repo]/index.astro`, not in either island, so the
  `slot="fallback"` skeletons occupy the same geometry as the hydrated islands and nothing
  reflows on hydration. DOM order is details-then-README at every breakpoint;
  `lg:flex-row-reverse` moves the sidebar right without reordering the document, keeping the
  mobile stack (details first) as the reading/focus order too. `Layout.astro`'s `max-w-5xl`
  deliberately unchanged — the `transition:persist`ed header shares it, so a per-page width would
  misalign the content edge on every navigation. `repo-href.ts` gained `refsHref`; branch/tag
  counts now link to `/{repo}/refs`. No route, no `shellFor`/`shell_for` change, no API contract
  change.

## Next up

None queued — #9's v1 scope is fully built out (search: #25/#26/#27; stats: #28/#29; HTTP push
stays permanently excluded, not deferred, by the read-only invariant), and the build-chunk-size
candidate above is now resolved. Pick the next piece of work from the candidates below, or from a
fresh request.

### Candidates (not urgent, no particular order)

- `git blame --follow` (rename tracking) — noted as a possible follow-up when blame was built
  (DECISIONS.md #14/#20), never revisited.
- `git grep`/`git log` exec fallbacks for search/stats if either proves too slow on a large
  repository — both left this escape hatch for themselves (DECISIONS.md #26/#28).
- Commit log's `path` filter walk can be slow on paths that change rarely across a long history
  (noted when `commits.rs::log` was built) — no reports of this being a real problem yet.
- `--chart-2..5` in `global.css` are still unvalidated shadcn boilerplate (DECISIONS.md #29) —
  revisit with the `dataviz` skill's validator if the stats page (or a future one) ever needs a
  second chart series.
