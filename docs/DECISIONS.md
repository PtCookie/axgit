# Decisions (ADR-lite)

Standing decisions, in numeric order, kept as stable anchors: source comments across `api/` and
`web/` cite them as `docs/DECISIONS.md #NN`, so numbers are never reused or renumbered. Each entry
records the decision and the constraints it still imposes — not the session that produced it. To
reverse one, don't delete it; leave a "Superseded by #N" note.

## #1 Replace Cgit with a custom frontend

Reimplement two roles — a read-only web UI and Smart HTTP clone — in a new stack. Push stays with
the existing git-server over SSH, so the web app has no auth or write path.

## #2 Backend: Rust + git2 (libgit2) + axum

- Rust chosen by preference; performance is a non-issue at this scale (the bottleneck is git object
  access and caching). git2 is stable; gitoxide's API is still in flux.
- axum + tower for the web layer.
- **Hybrid policy**: heavy operations (archive, upload-pack) use git binary exec, everything else
  is in-process git2.

## #3 Frontend: Astro (static) + React + shadcn/ui + Tailwind

No SSR adapter. Dynamic data is fetched client-side from React islands. Adding SSR would be a
separate decision reversing this one — it changes the deployment shape.

## #4 Monorepo + single container

web and api are tightly coupled through the API contract, so they live in one repo with atomic
commits. A single Rust binary serves static files + API + Smart HTTP, replacing nginx/fcgiwrap.

## #5 Preserve compatibility with existing data sources

- Metadata: read the repo config's `[cgit]` section; an `[axgit]` section takes precedence. Avoids
  having to modify git-server's `git-init`/`post-receive`.
- Last activity: agefile `info/web/last-modified`, falling back to the HEAD authordate.

## #6 Caching: cgit's TTL approach, with an improved validator

Response cache key `(repo, endpoint, params)`; validator = HEAD sha + agefile mtime. Sha-pinned
responses are immutable. (api/README.md#caching)

- The validator is a git2 open, then `head().target()` compared against the agefile's raw
  `SystemTime` (`repo/meta.rs::Validator`). Parsing `.git/HEAD`/`packed-refs` directly was rejected
  as fragile around symbolic refs, and the open cost is low.
- Caching lives in a **shared handler helper** (`handlers/mod.rs::cached_response`), not tower
  middleware: immutability is only known after ref resolution, param normalization and content type
  differ per endpoint, and error responses must never be cached.
- Strong ETag derived from the validator (format is not part of the contract). Request coalescing
  (`get_with`) is deliberately unused — it doesn't fit validator invalidation.
- **TTL (default 300s) is a safety net**, a staleness ceiling for out-of-band changes the validator
  can't see (manual config edits, a non-HEAD push without hooks). Per-entry body cap 1 MiB, total
  capacity tracked in bytes (default 32 MiB).
- Exceptions: the repo list uses `ScanCache` (TTL) + a body-hash ETag; raw is excluded (large
  binaries); archive is streamed, so it gets a weak ETag and no cache entry.

## #7 Toolchain

- **pnpm** workspace; `web` is a package, JS commands run from the root as `pnpm --filter web
  <script>`. api is Rust and stays outside the workspace.
- **lefthook** for git hooks — one binary covering both Rust and JS.
- Testing: web uses vitest (browser mode, `@vitest/browser-playwright` + `vitest-browser-react`) +
  Playwright e2e; api uses cargo test with fixture-repo integration tests.
- Commits: Conventional Commits, English.

## #8 API style

- REST JSON under `/api/v1`. `api/README.md` is the normative contract; the OpenAPI spec is generated
  from code (#15).
- Commit author emails are never exposed — only a hash, used as the avatar seed.
- Pagination uses a cursor (see #37 for its current form).

## #9 v1 scope

Included: repo list/summary/refs/log/tree/blob/raw/README/commit/diff/archive/Atom feed/Smart HTTP
clone, plus blame. Excluded at the time (all since built): stats, repository search. HTTP push is
permanently excluded.

## #10 TLS is delegated to a separate reverse proxy container

The axgit container serves HTTP only. TLS terminates in a separate reverse-proxy service with the
certbot volume mounted there. Consequence: absolute URLs are reconstructed from `X-Forwarded-*`
headers (#12).

## #11 Cgit filter replacements (frontend rendering)

Astro's built-in Shiki/markdown are build-time only, so runtime data is rendered by client
libraries:

- Syntax highlighting: Shiki client-side (lazy-loaded grammars, skipped for large files).
- Markdown README: react-markdown + remark-gfm + rehype-sanitize — sanitizing is mandatory, repo
  content is untrusted input.
- reStructuredText/man: **not rendered**, shown as plain text. No JS renderer exists and a
  docutils-scale dependency was rejected.
- Commit message linkification: regex-based, in React.
- Avatars: generated locally from an email hash (DiceBear), never Gravatar — no external requests.

## #12 archive/feed implementation

- archive uses **`git archive` exec** (#2's hybrid policy). The ref is resolved server-side and
  **only the full sha reaches the command line**, so there is no injection surface. stdout streams
  chunked via `tokio-util`'s `ReaderStream`; a separate task drains stderr and reaps the process.
- The Atom feed's XML is **generated by hand** with escaping helpers — a runtime XML crate isn't
  worth it for one fixed-structure document. quick-xml is a dev-dependency, used only to verify
  well-formedness in tests.
- No base-URL setting. Absolute URLs are reconstructed from `X-Forwarded-Proto`/`X-Forwarded-Host`/
  `Host` (a consequence of #10). Entry `<id>`s use `urn:sha1:{sha}` so they stay stable across
  hosts and feed readers don't show duplicates.

## #13 Smart HTTP: spawn `git upload-pack --stateless-rpc` directly

- Spawns upload-pack directly rather than going through `git http-backend` (CGI), reusing archive's
  `tokio::process` + `ReaderStream` + reaper pattern. Because the process is pinned to upload-pack,
  receive-pack is **structurally** unreachable — the read-only invariant is enforced by structure,
  not review.
- Only the git dir path resolved by `open_named` reaches the command line (same principle as #12).
  protocol v2 works by sanitizing the `Git-Protocol` request header into the `GIT_PROTOCOL` env var.
- A gzip'd request body is **fully buffered and decompressed synchronously with flate2** inside
  `spawn_blocking`; negotiation data tops out at a few MB even on huge repos, so streaming gunzip
  would add complexity for nothing. Limits: 8 MiB compressed (`DefaultBodyLimit`), 64 MiB
  decompressed (zip-bomb guard).
- stdin is written from a separate task to remove write/read deadlock risk; stdout streams.
- Smart HTTP responses are always no-cache.

## #14 blame: git2 `blame_file` (not exec)

- git2 `blame_file`, not a `git blame --line-porcelain` parser: api/README.md already lists blame
  under git2, the path never becomes a command-line argument, and git2 hands back hunk structs
  directly. If libgit2 proves slow on large histories, an exec fallback is reconsidered then.
- Binary/over-1-MiB detection reuses the blob endpoint's limit (`blob::BLOB_CONTENT_LIMIT`, shared
  via `classify`) — "this file can't be viewed" must not differ between blob and blame.
- Each range carries the commit `summary`; the commit is already fetched, and it saves the frontend
  one commit-detail call per range.
- Rename/copy tracking: see #36, which corrects this entry's original premise.

## #15 OpenAPI spec generated from code via utoipa

- **A hand-written `openapi.yaml` is rejected** — it becomes a second source of truth and drifts.
  The spec is generated from `#[utoipa::path]` + `#[derive(ToSchema)]` and committed to
  `openapi.json`. `api/tests/openapi_test.rs` checks both snapshot equality and "routed operations
  == spec operations", so a missing annotation fails CI. Regenerate with
  `AXGIT_UPDATE_OPENAPI=1 cargo test --test openapi_test`.
- **`utoipa-axum`'s `OpenApiRouter` auto-collection is not used.** tree/blob/raw/blame/archive are
  one `{*rest}` catch-all in axum with the ref/path boundary resolved at request time;
  auto-collection would document `/tree/{rest}`, which contradicts api/README.md. Paths are written by
  hand in `api/src/openapi.rs` — dual maintenance with the router, covered by the operation-list
  test.
- **Swagger UI ships via `utoipa-swagger-ui`'s `vendored` feature** (`/swagger-ui`): assets live in
  the binary, so neither build nor runtime makes external requests and the self-hosted environment
  works offline. Costs a few MB; no on/off flag, since the API is read-only and unauthenticated.
- Closed string sets are real enums (`DiffStatus`, `EntryKind`, `LineOrigin`, `ReadmeFormat`), with
  serde renames keeping JSON byte-identical. `#[schema(required = true)]` on always-serialized
  `Option` fields keeps generated types `field: T | null`, not `field?: T | null`.
- Error bodies are `ErrorResponse`/`ErrorBody` structs, not `serde_json::json!` literals, so the
  spec cannot drift from real responses.
- web generates `web/src/lib/api/types.ts` from `openapi.json` via `openapi-typescript`
  (`pnpm gen:types`); it is a generated file, never edited, and excluded from eslint.

## #16 Static build + SPA fallback

Superseded by #17. One note survives: in axum, routes that are `nest`ed or merged inherit the outer
`fallback_service`, so the api router needs an explicit JSON 404 fallback — otherwise
`/api/v1/bogus` answers with the HTML shell instead of a JSON error.

## #17 Prerendered page shells per route shape (refines #16)

- **Astro pages are the routing layer.** `src/pages/[repo]/*.astro` is prerendered once under a
  reserved `getStaticPaths` param (`__repo__`, `web/src/lib/shell.ts`), producing
  `dist/__repo__/...` shells alongside `dist/index.html` and `dist/404.html`. `output: "static"` and
  the single-container shape (#3, #4) are unchanged.
- **The server maps path *shape* → shell** (`api/src/shell.rs`). `ServeDir` serves real files first
  (`_astro/*`, favicons); what remains is matched by segment slice, and anything unmatched gets
  `404.html` with a real 404 — unmatched paths never answer 200. The `{repo}` segment is matched but
  never used to build a filesystem path, so it carries no traversal surface.
- **There is no client-side router.** Per-page chrome — repository heading, tab bar, active tab,
  `<title>` — is static HTML from `RepoLayout.astro`/`RepoNav.astro`. Only the repository *name* is
  unknowable at build time, so one `is:inline` script fills the heading, tab `href`s and
  `document.title` from `location.pathname`. `is:inline` rather than a bundled `<script>` because
  bundled scripts are deferred and would flash an empty heading.
- **Data islands stay `client:only="react"` with `slot="fallback"` skeletons.** `client:load` is
  rejected: the shell is built with a placeholder param, so a hydrated island would receive
  `"__repo__"` as a prop and have to re-derive the repository in an effect — extra render passes,
  broken component tests, and a hydration-mismatch footgun.
- **Cost: the mapping table existed three times** — `api/src/shell.rs::shell_for`,
  `web/src/lib/shell.ts::shellFor` (which also drives the `astro dev` middleware), and the
  `src/pages/` tree. Adding a route meant touching all three. Single-sourcing would require SSR,
  which #3 rules out — #88 closes most of the gap without one, by emitting the table itself from
  the build instead of hand-duplicating it in Rust.
- **A repository literally named `__repo__` is shadowed** — its requests hit the shell files
  directly. Benign today; only a real collision if a future route shape lacks a shell file.

## #18 Web `?ref=`-only ref selection, anchor-based log pagination, DiceBear identicons

- **Ref selection on the web is `?ref=` only.** Path segments after `/tree`, `/blob`, `/log` are
  always the file/commit path, never a candidate ref. Reimplementing the API's longest-match
  (`repo/resolve.rs::resolve_ref_path`) in the client was rejected: it would require fetching the
  refs list before rendering any file/log page. This is what keeps `shellFor`/`shell_for` pure
  segment-count matches — they never resolve a ref boundary.
- **Log pagination is a plain anchor (`Older →`), not client state** — consistent with having no
  client-side router. Only a forward link is rendered; the cursor is one-directional and the browser
  back button covers the rest, matching cgit's own pager UX.
- **Avatars: `@dicebear/collection`'s `identicon` style**, seeded from `email_hash`, rendered as a
  `toDataUri()` `<img>`. `@dicebear/core` is pinned to `^9.4.3`, not `^10`: `@dicebear/collection`
  declares a peer range of `^9.0.0` and several bundled style packages import an `escape` helper v10
  no longer exports, which breaks Vite's dependency pre-bundling. Revisit once `@dicebear/collection`
  publishes a v10-compatible release.

## #19 `/{repo}/tree` + `/{repo}/blob` pages, Shiki highlighting

- **`shellFor`/`shell_for` (#17) match a segment slice, not a fixed arity** — tree and blob paths
  have unbounded depth. `/{repo}/tree` (root, no path) is valid; `/{repo}/blob` with no path is not
  and 404s.
- The `astro dev` `shellFallback` middleware decides "is this an app route" by an actual `public/`
  file-existence check (normalized and prefix-checked against `public/` to rule out `..` escaping),
  not by extension. An extension heuristic breaks blob paths like `/{repo}/blob/src/main.rs`, which
  contain a dot but are app routes — production (`api/src/shell.rs`) has no such restriction.
- **Shiki via `shiki/core` + `shiki/engine/javascript`** (JavaScript RegExp engine,
  `forgiving: true`), not the Oniguruma/WASM engine — avoids shipping a ~500 KiB `.wasm` asset, at
  the cost of reduced grammar accuracy for a few complex languages. Both `github-light` and
  `github-dark` are tokenized together (`codeToTokens` with `themes: {light, dark}`) and each
  token's `htmlStyle` (a `color` plus a `--shiki-dark` custom property) is used as a React inline
  style, completed by `.dark .shiki-code span { color: var(--shiki-dark) !important; }` in
  `global.css`.
- **Grammars lazy-load per file** (`lib/format/highlight.ts`'s `LANG_LOADERS`, keyed by extension);
  unmapped languages render as plain text. Highlighting is skipped above **512 KiB or 5000 lines**.
  Tokens render as React nodes, never `dangerouslySetInnerHTML` — repository content is untrusted,
  the same rule `lib/format/linkify.tsx` follows.
- `ref` stays `?ref=`-only (#18); when absent the literal string `HEAD` is passed as the API path's
  `{ref}` segment rather than fetching the refs list to resolve a default.

## #20 `/{repo}/blame` page

- **The blame response carries no file content**, so `BlameView` fetches `getBlame`/`getBlob` in
  parallel and lays the ranges over `blob.content`.
- **`CodeBlock.tsx` takes an optional `gutter` prop** rather than there being a blame-specific code
  viewer: a `(GutterCell | null)[]` indexed like the content's lines, where `GutterCell.rowSpan`
  merges one `<td>` down over a whole blame range. Omitting `gutter` renders exactly as before the
  prop existed. Trailing display-only lines (the empty element `content.split("\n")` produces after
  a final newline) are padded with a blank one-row cell so the column stays aligned.
- **Gutter content is compact, cgit-style**: 7-char sha, relative time, author name; the commit
  summary is a `title` tooltip only, and there is no avatar, to keep the code column wide.
- **Entry is blob-only, no dedicated nav tab.** `RepoLayout.astro`'s active-tab mapping treats
  `blame` like `blob` (highlights "Tree").
- Repo-page hrefs live in `web/src/lib/repo-href.ts` (`treeHref`/`blobHref`/`blameHref`), not
  inline in components.

## #21 README rendering + archive/feed links

- **react-markdown + remark-gfm + rehype-sanitize, no `rehype-raw`** (#11). Raw HTML in a README is
  dropped rather than sanitized-and-kept: re-introducing it would mean trusting
  `hast-util-sanitize`'s schema to catch everything, and repository content is untrusted — the same
  reason Shiki tokens and `linkify.tsx` never use `dangerouslySetInnerHTML`.
- **Relative links and images are rewritten to repository URLs** (`lib/markdown-url.ts`'s
  `isExternalUrl`/`resolveRepoPath`). Without this every relative `./docs/x.md` or
  `images/logo.png` 404s, since the browser resolves them against the page URL, not the repository
  tree. `<a href>` maps through `treeHref`/`blobHref`, `<img src>` through `rawUrl`. Anything
  resolving outside the repository root, or already absolute, passes through untouched.
- **`ReadmeView` is its own island**, not folded into `RepoSummary` — a 404 here is the endpoint's
  normal "nothing to show" outcome, and a separate fetch keeps it out of `RepoSummary`'s error
  state.
- **Markdown code fences get Shiki highlighting** through `highlightFence`/`languageForFence`,
  sharing `highlightCode`'s extension-alias table.
  - **Pitfall**: with a `code` override set, react-markdown makes a hast `<code>` node's React
    element `type` the *provided component function*, not the string `"code"` — so `pre`'s "is my
    child a fenced block" check must compare against that component's own reference (hoisted to
    module scope as `InlineCode`). `children` passed to `pre` is also always an array, so it must be
    unwrapped before `isValidElement`. Getting either wrong silently falls back to plain text with
    no error, which is why `ReadmeView.test.tsx` asserts on the rendered
    `pre.shiki-code > span[style]` rather than on fence text.
- **archive/feed are HEAD-only links on the summary page** — no per-ref archive UI on `/refs`.
  `archiveUrl`/`feedUrl` are link builders, never `fetch`ed. Both hide with the rest of the
  empty-repository branch (`head === null`).

## #22 Container image: 3-stage alpine/musl build, `safe.directory = *`, pinned base images

- **alpine/musl over debian-slim**: `git2` is `default-features = false` (no ssh/https transports),
  so there is no openssl-sys/libssh2-sys to fight with on musl. `libgit2-sys`'s build.rs finds no
  system libgit2 on alpine and builds the vendored source with `cc`; the only added builder package
  is `musl-dev`. The result is fully static (musl defaults to `crt-static`), so the runtime stage
  needs no shared libgit2/zlib. With `utoipa-swagger-ui`'s `vendored` feature its build.rs never
  hits its download code path either — the build is network-free past `cargo fetch`/`pnpm install`.
- **Runtime packages are `git` + `ca-certificates` only** — no `tzdata`: `repo/meta.rs` only builds
  `Zoned` values from `TimeZone::UTC` or `TimeZone::fixed(offset)`, so jiff never consults the
  system tzdb.
- **`/etc/gitconfig` sets `[safe] directory = *`, and the image runs as a non-root uid 10001.**
  `/srv/git` is a read-only bind mount owned by the git-server container's uid, which trips the
  ownership check in both `git` and libgit2 — both read the system gitconfig, so one file covers the
  `git archive`/`upload-pack` exec paths and the git2 `Repository::open` paths at once. Rejected:
  running as root (defeats the read-only posture, and would still need ownership handling once
  dropped to a real user) and `git2::opts::set_verify_owner_validation(false)` (covers libgit2 only,
  so the gitconfig would still be needed — a second mechanism for nothing). Trusting every directory
  is scoped to this single-purpose read-only container.
  - Note for anyone re-testing this: the negative control can't be reproduced on macOS Docker
    Desktop, whose virtiofs bind mount reports files as owned by the accessing container's uid. On a
    Linux host — the real deployment target — a `:ro` bind mount preserves the host uid and the
    mismatch is real.
- **BuildKit cache mounts** for pnpm's store and cargo's registry + `api/target`. Cache mount
  contents don't persist into the image layer, so the release binary is `cp`'d out to `/axgit`
  inside the same `RUN` that builds it.
- **Base images are pinned to a minor version** and collected into `ARG`s at the top of the file; a
  floating tag would let the image drift on every rebuild. `apk` packages aren't individually pinned
  (an alpine minor's repository only receives patches), pnpm is pinned by the root `package.json`'s
  `packageManager`, and crates by `Cargo.lock` + `--locked`. Digest pinning is deliberately not
  used — bumping the minor tags is a deliberate, separate commit either way. The `ARG` refs were
  later registry-qualified and the file renamed to `Containerfile`; see #80.

## #23 3-way theme selector (System / Light / Dark)

- **Two `is:inline` scripts; the resolve itself is not React.** The pre-paint half must be a
  synchronous classic script in `<head>` (`Layout.astro`) — a bundled `<script>` is deferred and
  flashes the light theme on every load. `<html data-theme>` carries the stored *preference*
  (`system`/`light`/`dark`); the `dark` class carries the *resolved* value, which is what
  `@custom-variant dark` and the `.dark {}` token block key off. Neither is ever server-rendered:
  the shells are prerendered once per route shape and served `no-cache` (#17), so a cookie-driven
  server-rendered theme is structurally unavailable — the mechanism has to be client-side.
- **The toggle is a `ThemeToggle` React island** (`client:only="react"` + static fallback). Its
  trigger renders all three icons unconditionally and `global.css` reveals only the one matching
  `<html data-theme>`, so the correct icon shows in the prerendered fallback before hydration.
  Icons are Phosphor components (`iconLibrary: "phosphor"` in `components.json`).
- **shadcn `dropdown-menu` (Base UI `Menu`) with `RadioGroup`/`RadioItem`.** `MenuRadioItem`
  renders `role="menuitemradio"` with `aria-checked` natively — no custom ARIA wiring. Selecting an
  item deliberately does not close the menu (Base UI's default), which doubles as a theme preview.
- **`global.css`**: `color-scheme` is keyed to the resolved class rather than
  `prefers-color-scheme`, so a manual override reaches native UI; `@custom-variant dark` is widened
  from shadcn's `&:is(.dark *)` to `&:is(.dark, .dark *)` because the class lands on `<html>` itself,
  which the descendant-only form excludes. No transition-suppression rule is needed — nothing
  visibly animates on a switch.
- **`.dark --primary` is `oklch(0.72 0.13 166)`, not shadcn's generated value**, which was *darker*
  than its light counterpart and left every `text-primary` link, the active-tab underline and the
  `CodeBlock` line highlight under ~3:1 against `--background`.
- Diff-line backgrounds in `CommitView.tsx` carry explicit `dark:` variants; everything else is on
  semantic tokens and flips for free.
- **Testing note**: theme behavior is e2e-only (`web/e2e/theme.spec.ts`). The head script can't be
  extracted into an importable module without bundling it into a deferred script — the exact flash
  the design prevents. Two Playwright/CDP quirks are worked around in the test file, not the app:
  `colorScheme` emulation updates `matchMedia(...).matches` without reliably dispatching the
  `"change"` event, and each `matchMedia()` call mints a distinct object, so the OS-tracking test
  pins the query to one shared object via `addInitScript`. A screenshot-based no-flash assertion is
  rejected — nothing observable runs before first paint, and Playwright drives `astro dev`, whose
  inline-`<style>` FOUC the static build doesn't have. The structural equivalent is asserted
  instead.
- **Known limitation**: README images are arbitrary repository content, so a transparent PNG with
  dark artwork can be invisible in dark mode. The usual `<picture>` + `prefers-color-scheme` fix is
  unavailable because `rehype-sanitize` without `rehype-raw` strips it (#21).

## #24 Client-side routing via Astro's `<ClientRouter />` (refines #17)

`<ClientRouter />` in `Layout.astro` turns same-origin link clicks into client-side `<body>` swaps
with a short fade on `<main>`. This is Astro's own router, not a return to the hand-rolled one #17
deleted.

- **Why SPA mode over native cross-document view transitions**: the CSS-only
  `@view-transition { navigation: auto }` approach still does a full page load per navigation, so
  it wouldn't touch the actual cost — React runtime, Shiki's per-language dynamic import and
  DiceBear all re-executing on every Summary/Tree/Log/Refs tab switch.
- **`<main>` is not persisted; `<header>` is.** Every data island is `client:only="react"` and reads
  `location` exactly once, at mount (`lib/repo-param.ts`). Persisting `<main>` across a same-shell
  navigation would keep the *old* island alive showing stale data instead of remounting against the
  new URL. `<header>` is identical everywhere and holds only `ThemeToggle`, so persisting it avoids
  re-flashing that island's skeleton on every navigation.
- **The theme resets on every swap unless explicitly reapplied.** Astro's `swapRootAttributes`
  replaces `<html>`'s entire attribute set with the incoming document's, and per #23 the prerendered
  shell's `<html>` carries neither `data-theme` nor `class`. `applyStoredTheme()` is therefore
  called on initial load *and* on every `astro:after-swap`.
- **`astro:after-swap`, not `data-astro-rerun`, for both shell-mutating scripts** (theme,
  `window.__axgit.fillRepoShell`). `data-astro-rerun` executes inside `runScripts()`, which fires
  only after the transition's `updateCallbackDone` resolves — i.e. after paint. `astro:after-swap`
  fires from inside the DOM-swap step, before that paint, which is the guarantee both scripts rely
  on. (Verified against Astro's `dist/transitions/router.js`, not the docs.)
- **The repo-shell fill-in lives in `Layout.astro` as `window.__axgit.fillRepoShell`**, keyed off a
  `[data-repo-name]` query rather than `document.currentScript`. Two forced constraints:
  `document.currentScript` is `null` when Astro re-executes a script post-swap, so the title suffix
  travels as `data-title-suffix` on the heading element; and the `astro:after-swap` listener must be
  registered by a script present on *every* page, so the first navigation *into* a repository page
  already has one. `RepoLayout.astro` keeps a one-line call for its own initial load.
- **`prefetch.prefetchAll: false`**, overriding the default `<ClientRouter />` sets. Every
  `/{repo}/blob/*` (and tree/blame/commit) maps to one byte-identical shell served `no-cache` (#17),
  so hover-prefetching re-fetches the same HTML for no benefit — the per-page data always arrives
  afterward from `/api`.
- **The `astro dev` middleware also matches `Sec-Fetch-Dest: empty`.** `ClientRouter`'s `fetchHTML`
  sends `Accept: */*` with no adapter, so an `Accept: text/html`-only match 404s every client-side
  navigation under `astro dev`/Playwright. Production matches on path shape alone and needed no
  change.
- **`::view-transition-old(root)`/`-new(root)` animations are disabled in `global.css`** — the root
  snapshot covers the persisted header and tab bar, and would cross-fade underneath `<main>`'s own
  animation. No separate `prefers-reduced-motion` handling: `<ClientRouter />` ships one.
- Regression coverage lives in `web/e2e/`: an explicit Dark choice surviving a client-side
  navigation, and a client-side marker surviving one — the only signal that a test exercises
  client-side routing rather than a silently downgraded full reload onto the same URL.

## #25 Repository list filter (client-side, `?q=`)

A client-side filter over the repository list `/` already fetches — no API change, no new route.

- **Matching rule** (`web/src/lib/repo-filter.ts::filterRepos`): the query splits on whitespace;
  a repository matches only if *every* term (AND) is a case-insensitive substring of *at least one*
  of `name`/`description`/`owner`/`section`. `null` fields are skipped. An empty query returns the
  input array unchanged, so callers can detect "no filter" by reference equality. Filtering happens
  before `groupBySection`, so an empty section simply doesn't render.
- **`?q=` via `history.replaceState`, not `pushState`** — a history entry per keystroke would break
  the back button. This doesn't interact with `<ClientRouter />` (#24), which reads `location` only
  at link-click time. The initial query is read via `lib/repo-param.ts::paramFromSearch`, so a deep
  link lands already filtered.
- No debounce: filtering an in-memory array needs no async work to throttle.
- **Content search is deliberately not this** — see #26.

## #26 Repository search API: git2 in-process scan, not exec or an index

`GET /api/v1/repos/{repo}/search?q=&type=&ref=&limit=` — searching *inside* a repository.

- **git2 in-process scan, not `git grep` exec, not a persistent index.** An index would be this
  app's first piece of mutable, persistent state in an otherwise stateless, read-only container — a
  deployment-shape change, rejected outright. Between the exec-free options git2 won: `handlers/
  mod.rs::cached_response` is synchronous and git2 fits it directly, an exec would force it into an
  async-closure shape for one caller; git2 also allows an *exact* byte/file/commit budget rather
  than only a process timeout, and no query string ever reaches a command line. If a large
  repository proves this too slow, a `git grep` exec fallback is the escape hatch (as in #14).
- **One endpoint with a `type` selector**, not three routes: all types share the "resolve a ref,
  walk something, cap the results" shape and one response envelope, and cgit presents them as one
  box too.
- **Two independent budgets.** `limit` (1–100, default 50) caps *results*; a separate fixed scan
  budget caps *work regardless of matches* — 20,000 tree entries walked, 32 MiB of blob content read
  (checked via `Odb::read_header` first, so oversized blobs are never loaded), 10,000 commits walked
  for `type=message`. Either running out sets `truncated: true`. Content search reuses
  `repo/blob.rs::classify`, so search never surfaces what the blob view would refuse to render.
- **`q` is a fixed string, not a regex**, matched case-insensitively (ASCII fast path, falling back
  to full `to_lowercase`). Regex would reopen the unbounded-worst-case problem the budgets close,
  for a feature cgit's search doesn't offer either.
- Caching, immutability and the empty-repository carve-out follow commit-detail's and the commit
  log's existing rules exactly.
- Shared helpers: `parse_limit`/`DEFAULT_LIMIT`/`MAX_LIMIT` live in `handlers/mod.rs`;
  `repo/commits.rs::commit_info` is `pub(crate)` so `type=message` results reuse the exact log-entry
  shape rather than a parallel struct.

## #27 `/{repo}/search` page

- **Route** `[repo]/search.astro` follows `log.astro`: no path segments, all state in the query
  string, so `shellFor`/`shell_for` need only a two-segment match. It is its own nav tab, not a
  drill-down.
- **Plain `<form method="get">`, no controlled inputs, no client-side state.** `<ClientRouter />`
  registers a `submit` listener that intercepts same-origin GET forms exactly like anchor clicks
  (verified against Astro's `ClientRouter.astro` source), so no `onSubmit` handler is needed and the
  form degrades to a full-page GET without JS. Inputs are uncontrolled (`defaultValue` +
  `paramFromSearch`), matching the "props override, `location` is the default source" pattern.
- **`SearchView`'s loading state is lazily derived from the initial query**, not reset inside the
  fetch effect — same rationale as `CommitLog` (a live-instance prop change only happens in tests;
  production always remounts fresh), which also avoids `@eslint-react/set-state-in-effect`.
- `content`/`path` results group by file and link through `blobLineHref` (`#L{n}`, reusing
  `CodeBlock.tsx`'s anchor scheme); `message` reuses `CommitLog`'s row shape. A `truncated: true`
  response renders a visible banner — otherwise a capped result looks identical to a complete one.

## #28 Commit-statistics API: git2 revwalk, 12 fixed buckets anchored on the resolved commit

`GET /api/v1/repos/{repo}/stats?ref=&period=&limit=`.

- **git2 in-process revwalk, not a `git log` exec, not a persistent index** — same reasoning as #26,
  with an exact commit budget (`MAX_SCANNED_COMMITS = 20_000`).
- **The window is anchored on the resolved commit's authordate, not the request time.** cgit anchors
  on "now", but that would make every response time-dependent and therefore ineligible for the
  immutable full-sha caching rule. Anchoring on the commit makes a full-sha request deterministic,
  so it gets the same `public, immutable` treatment as commit detail/diff/blame.
- **12 buckets, fixed, regardless of `period`** — `period` changes each bucket's duration, not how
  many there are, so the response shape and the web table's column count stay stable.
- **Bucket boundaries are calendar-aware, computed with jiff `Span`s**, not fixed-duration
  arithmetic: month/quarter/year lengths vary, and bucket starts must land on the 1st. Week buckets
  start Monday. All bucketing is in UTC — the anchor's original offset is discarded once it has
  picked the UTC instant, so boundaries don't depend on where the author's clock was set.
- **A commit authored after the window's end clamps into the last bucket rather than being dropped**
  — out-of-order authordates happen when an old branch is merged late. Older commits are excluded.
  There is no early-exit heuristic, because revwalk order isn't strictly chronological across
  merges; only the scan budget bounds the walk.
- **`authors[].buckets` is a parallel array to the top-level `buckets`**, so the frontend never has
  to keep a second boundary/label scheme in sync.
- **`limit` caps only the `authors` rows, never the bucket totals** — `buckets[].commits` always
  reflects every commit in the window, and `author_count` reports the full distinct count so the UI
  can say "top N of M". `truncated` is set by either cause, matching search's precedent.
- Emails are never exposed; the author breakdown reuses `signature_info`/`CommitAuthor`.

## #29 `/{repo}/stats` page: Recharts (via shadcn), dataviz-validated chart color

- **Route** `[repo]/stats.astro` follows `search.astro`'s shape; it is its own nav tab.
- **Recharts is installed via `pnpm exec shadcn add chart`**, not `pnpm add recharts` — the registry
  item pins the version and pulls `registryDependencies: card`, so `ui/chart.tsx` and `ui/card.tsx`
  both land the vendored way. The vendored `chart.tsx`'s remaining lint *warnings* are left as
  shadcn generated them ("vendored, modify only what's broken").
- **`--chart-1` was validated, not eyeballed.** Its shadcn boilerplate values were byte-identical
  across light/dark and both failed: light read **1.44:1** against the surface (invisible as a bar
  fill) and dark's lightness (OKLCH L 0.855) sat above the dark band (0.48–0.67). Fixed by reusing
  `--primary`'s hue/chroma (165.612 / 0.118) at two lightness steps — L 0.508 for light, a new
  L 0.6 for dark. `--primary`'s own dark value (L 0.72) is tuned for *text* contrast, not a chart
  mark's dark band, so it could not be reused as-is. `--chart-2..5` were left untouched and
  unvalidated here; see #73.
- **`minPointSize={2}` on the `<Bar>` is required, not cosmetic**: without it Recharts omits the
  rectangle entirely for a zero-value bucket, leaving no hover/tooltip hit target for that month.
- **The period switcher is four plain links (`statsHref`), not a form** — a fixed 4-way pick needs
  no no-JS form fallback, and `<ClientRouter />` already intercepts link clicks.
- **The author table doubles as the chart's required table view**: every value a mark shows must be
  reachable without the mark. `authors[].buckets` becomes one column per bucket with a `TableFooter`
  "Total" row, so nothing is chart-only. `author_count` vs `authors.length` renders as
  "Showing top N of M".

## #30 Lazy-load Recharts, react-markdown, and the theme menu behind `React.lazy`

The chunk that trips Vite's 500 kB warning is Shiki's `cpp` grammar (637 KB), which is already lazy
and left alone. The real cost was three *eagerly* loaded chunks — `ThemeToggle` (137 KB, every
page), `StatsView` (345 KB) and `ReadmeView` (145 KB) — now split behind `React.lazy`.

- **`ThemeToggle`/`ThemeMenu`/`theme.tsx` three-way split.** `@base-ui/react`'s Menu (~121 KB) was
  the only thing in the app pulling in `@base-ui/react` at all, so moving it behind `import()` moved
  the bytes outright. `theme.tsx` holds the leaf state/icons shared by the eager trigger and the lazy
  menu; **neither of those two imports the other**, since a cross-import pulls the lazy side's
  weight back into the eager chunk.
  - Uses base-ui's **`defaultOpen`**, not controlled `open`/`onOpenChange`: `useInitialOpenSync`
    initializes the popup open on first render and then owns every close path itself, so there is
    nothing to resync from the eager side. A trigger mounting in the same commit as an already-open
    popup is claimed pre-paint by `registerTrigger`, so there is no anchor-position gap. (Verified
    against `@base-ui/react` 1.6.0 source.)
  - The trigger's `onClick` wraps `setMenuRequested(true)` in `startTransition` —
    `@astrojs/react`'s own `startTransition` only covers the initial `client:only` mount, not a
    later click that suspends a boundary. `onPointerEnter`/`onFocus` warm the import ahead of the
    click, and the pre-interaction button and the `Suspense` fallback are the same `PlainTrigger`
    element, so nothing flashes even if the fallback commits.
  - Result: 137 KB → ~2 KB eager, ~119 KB fetched on first hover/focus/click.
- **`StatsView`/`StatsChart` split, with pre-warming.** `StatsView` fires
  `void import("./StatsChart")` in parallel with its `getStats` fetch, since nearly every response
  renders a chart and serializing the chunk behind the API round trip would be pure latency. The
  `Suspense` fallback is pixel-matched to `ChartContainer`'s sizing and the page skeleton's middle
  block, so there is no layout shift whichever resolves first. 345 KB → ~5 KB + a ~333 KB chunk.
- **`ReadmeView`/`ReadmeMarkdown` split, deliberately *not* pre-warmed** — the opposite policy, on
  purpose: a repository with no README, or an `rst`/`plain` one, should never download
  react-markdown at all, and `format` isn't known until the `/readme` response lands.
  `ReadmeMarkdown.tsx` takes the entire markdown surface as one unit, keeping `InlineCode` and the
  `pre` override's `child.type === InlineCode` identity check (#21) in the same module — splitting
  those across files would reintroduce that exact bug with no type error to catch it. `components`
  is `useMemo`'d on `repo`. 145 KB → ~2 KB + a ~140 KB chunk fetched only for a markdown README.
- **Confirmed non-issues**: `@phosphor-icons/react`'s barrel import already tree-shakes correctly;
  the `cpp` grammar chunk only downloads when a `.cpp`/`.hpp` file is viewed.

## #31 `/{repo}` summary: two-column layout, description + metadata in a right sidebar

- **The grid lives in `pages/[repo]/index.astro`, not in either island.** `RepoSummary` and
  `ReadmeView` stay two independent `client:only` islands, with the two column boxes as plain Astro
  elements around them, so the geometry is identical before, during and after hydration and the
  fallback skeletons never snap from full-width into a sidebar. Owning the grid inside a merged
  React component would force one combined fetch/skeleton and pull `ReadmeMarkdown`'s module graph
  into the summary chunk regardless of README format, defeating #30.
- **`lg:flex-row-reverse`, not `order-*` or `flex-col-reverse`.** DOM order is
  details-then-README at *every* breakpoint; only desktop placement changes. `flex-col-reverse` +
  `lg:flex-row` is visually equivalent but reorders the *mobile* DOM, forcing keyboard and
  screen-reader users through a long README before the details block rendered above it (WCAG
  1.3.2/2.4.3).
- **`lg` (64rem), not `md`.** At `md` the `max-w-5xl` box leaves ~416px for the README — too narrow
  for code fences and GFM tables. 64rem is exactly `max-w-5xl`, so two columns start precisely where
  the container stops growing.
- **`min-w-0` on the README wrapper is required.** Flex items default to `min-width: auto`; one long
  unbroken `<pre>` line would otherwise push the sidebar off-screen. Left unprefixed, since the
  mobile column has the same risk.
- **`Layout.astro`'s `max-w-5xl` stays.** The `<header>` shares that width and is
  `transition:persist`ed (#24), so a page-only width would offset the content edge from the unmoving
  header and jump on every tab click. Widening is a global decision for both elements together.
- **Branches/tags collapse into one "Refs" `<dt>`** whose links carry the full phrase as their
  accessible name (`"3 branches"`), not a bare count (WCAG 2.4.4). No `#branches`/`#tags` fragment:
  `RefsView` is `client:only`, so its sections don't exist when the browser processes a fragment.
- **`components/ui/card.tsx` is still unused** — vendored only as a `shadcn add chart` registry
  dependency (#29). Its `rounded-4xl`/`shadow-md`/`bg-card` look doesn't match this app's flat
  hairline-border surfaces.
- Icons are `aria-hidden="true"` — load-bearing, since tests look the download/feed links up by
  accessible name.
- `<aside aria-label="Repository details">` lives in the Astro page, so the `complementary` landmark
  exists in prerendered HTML and survives the island's loading and error branches.

## #32 The page fade is a plain CSS animation, not a view transition (fixes a Firefox squash, refines #24)

Firefox visibly compressed or stretched the page vertically on every client-side navigation;
Chromium was unaffected. `<main>` no longer carries a `view-transition-name`, and `global.css`
animates the element itself.

- **Why it distorted.** Naming an element makes the UA animate its `::view-transition-group` box
  from the old element's size to the new one's. Firefox scales the snapshot to fit that box;
  Chromium keeps the snapshot at its intrinsic block size and lets it overflow. Identical markup,
  opposite results — a rendering difference, so no duration/easing tweak would have helped.
- **`client:only` islands made it dramatic.** The group's target size is `<main>`'s height *at the
  DOM swap*, when `<main>` holds nothing but static fallback skeletons (#17); the real content lands
  milliseconds later, mid-transition. Note for anyone re-testing: headless Firefox does not
  reproduce the mis-scaled paint — a real window against a fixed reference ruler outside `<main>` is
  required to see it.
- **Overriding the pseudo-elements was rejected** (`object-fit: none`, an explicit `block-size` on
  `::view-transition-old/new`): that makes correctness depend on Firefox honoring an override for
  sizing it already renders differently from the spec default. Dropping the name removes the failure
  mode outright.
- **Every `::view-transition-*(root)` animation is off, group included.** With no named elements the
  page is a single viewport-sized `root` snapshot whose size cannot change, so the group animation
  is a 250 ms no-op holding a frozen snapshot on screen. `<ClientRouter />` still drives the swap
  through `startViewTransition` — that is what keeps `astro:after-swap` firing before paint, which
  both head scripts depend on — it just animates nothing.
- **Behavior**: the incoming page fades in over 0.18 s (`@keyframes axgit-page-in`); the outgoing one
  is gone once the swap commits. `<main>` is never persisted, so Astro inserts a fresh element every
  navigation and the animation restarts on its own — no class toggle, no hook. It also plays on a
  cold load, which the shared-element version could not.
- **`prefers-reduced-motion` needs its own rule now.** `<ClientRouter />`'s built-in media query
  governs only view-transition animations, not a plain CSS one.

## #33 Commit graph column laid out client-side, no walk-order change

The Log tab's leading graph column (cgit's ASCII DAG as inline SVG) needed no API change —
`CommitInfo.parents` was already in every `/commits` response.

- **The walk stays unsorted, deliberately** (`repo/commits.rs::log`). libgit2's `revwalk.c` sets
  `walk->limited = 1` for *any* sort flag — `GIT_SORT_TOPOLOGICAL` and even bare `GIT_SORT_TIME` —
  which makes `prepare_walk` run `limit_list`, draining the entire reachable history before the
  first commit is emitted. `GIT_SORT_NONE` keeps a lazy O(page size) walk; since the response cache
  keys on the cursor, N pages on a cold cache would otherwise mean N full-history walks. Committer
  order is enough for a correct graph: within a page a parent can never be emitted above its child,
  because the lazy walk only discovers a commit by popping an already-emitted child. The cost is
  that branches interleave more than `--topo-order` would.
- **One row per commit, no cgit-style `|\`/`|/` filler rows** — filler rows would leave the other
  columns empty and break `TableRow` hover/zebra semantics. Trade-off: a merge edge gets half a row
  (24px) of travel, so a wide lane jump reads as near-horizontal. Leftmost-free lane allocation
  (`web/src/lib/commit-graph.ts`) keeps most jumps to 1–2 lanes, and lanes never shift horizontally
  once assigned (a freed lane is left as a hole and reused), so every `through` line is a straight
  vertical.
- **Monochrome; merge-vs-normal is encoded as shape (hollow ring vs filled dot), not colour.** Do
  not "improve" this with per-lane hue without running the dataviz validator on `--chart-2..5`
  first (see #29, #73).
- **Hidden when `?path=` is set** — `touches_path` yields a subsequence of the true history, so
  edges between displayed commits would be arbitrary. Also hidden under `sm` so it can't crowd out
  the useful columns.
- **Lanes reset at each page boundary.** Encoding lane state into the cursor would break its opaque
  contract, and cgit resets per page too. `continuesAbove` draws row 0's edge to the top rather than
  showing a fake root when its children are on the previous page.
- **No new dependency** — the layout + renderer is ~190 lines, small enough not to need lazy loading.

## #34 Ref badges on the Log tab and commit detail, via a parallel client-side `/refs` fetch

- **Not the prefetch #18 rejected.** #18 rejected reimplementing the api's ref longest-match
  client-side to *resolve a URL* — a blocking dependency gating render on a second request. Badges
  are pure decoration: `lib/commit-refs.ts::useCommitRefs` fires `/refs` in its own effect, parallel
  to and independent of the page's own fetch. No API change.
- **Fails silently** — a rejected `/refs` request leaves the index empty; no error state, no retry.
  Badges are additive polish, not required data.
- **No HEAD/default-branch badge.** `GET /refs` returns branches and tags only and `default_branch`
  lives on the repo summary; a third request for a styling nuance isn't worth it. The default branch
  still appears as an ordinary branch badge.
- **Icon distinguishes kind, not colour** (`GitBranchIcon`/`TagIcon`) — same reasoning as #33.
- **Capped at 3 in the log table, uncapped in the commit detail header.** A heavily-tagged commit
  must not blow out the `h-12` row height #33's graph column relies on; the table shows the first 3
  (branches before tags, both name-sorted by the api) plus a `+N` link to `/{repo}/refs`.

## #35 cgit URL compatibility redirects

Old cgit bookmarks and any `.git`-suffixed page URL used to render a broken page: the shape matched
`shell_for` (which never validates the repo segment), so the shell served 200 and the island then
resolved the repo as literally `{repo}.git`, which the API 404'd.

- **Core shapes only, not full cgit coverage.** `/{repo}[.git]/commit/?id=` and `/{repo}[.git]/diff/`
  → `/{repo}/commit/{sha}`; `/{repo}[.git]/log/?h=` → `/{repo}/log?ref=`; bare `/{repo}.git` →
  `/{repo}`; any other `.git`-suffixed path has the suffix stripped, query preserved. cgit's
  `tree/{path}?id=` is **not** split into tree vs blob — telling those apart needs a git lookup the
  redirect layer doesn't have — so it only gets the generic strip. `plain/`, `atom/`, `snapshot/`
  are out of scope.
- **Redirect loops are ruled out structurally**: `cgit_compat::redirect_for`/`redirectFor` return
  `None`/`null` whenever the computed target would equal the request as-is, so anything already a
  valid native shape is left untouched.
- **Runs in `shell::serve_shell_or_redirect`**, after `ServeDir` and after the Smart HTTP / Swagger
  UI routes (real routes, matched before the fallback), so clone/fetch and the API are untouched.
- **No percent-decoding.** `id`/`h` are copied into the `Location` on their raw, still-encoded text,
  and `id` is constrained to `[0-9a-fA-F]{4,64}` first — nothing built here can carry characters the
  request didn't already contain, so there is no decode round trip and no header-injection surface.
- **Mirrored into the dev server** (`web/src/lib/cgit-compat.ts`, called from `shellFallback()`
  ahead of the `shellFor` rewrite), same dual-maintenance rule as `shellFor`/`shell_for`.

## #36 blame rename tracking (corrects #14's premise)

- **#14's premise — that libgit2's default blame has no rename tracking — was wrong.**
  `blame_git.c::find_origin` runs its own `git_diff_find_similar` with `GIT_DIFF_FIND_RENAMES`
  between a commit and each parent while walking, the same default-threshold match `git blame` uses.
  Verified by comparing a C program linked against libgit2 1.9.6 with `git blame --porcelain` across
  a plain rename, a rename + edit in one commit (including a move into a subdirectory), and a
  multi-hop rename chain: all matched line-for-line, `orig_path` chain included. No exec fallback
  was needed — rename tracking already worked, it just wasn't surfaced.
- **What is genuinely unsupported**: line-level move/copy tracking (`git blame -M`/`-C`).
  libgit2's `GIT_BLAME_TRACK_COPIES_*` flags exist in the header but are documented upstream as not
  implemented. That is the part that would need an exec fallback, and it remains unimplemented.
- **`BlameRange.orig_path: Option<String>`** — the path a hunk's commit had at that point in
  history, collapsed to `None` when it equals the blamed path or isn't valid UTF-8. Kept per-hunk
  rather than in the per-commit cache, since one commit can appear under different `orig_path`s in
  different files.
- **Web**: the gutter cell gets a marker next to the short sha when `orig_path` is set, linking to
  `blameHref(repo, orig_path, sha)`.

## #37 Commit log cursor is an offset token

- **Root cause of the dropped-commit bug** (confirmed against libgit2 1.9.6's `revwalk.c`):
  `GIT_SORT_NONE` is not an unordered walk — `revwalk_next_unsorted` pops from a pending list kept in
  commit-date order. The old cursor stored only the boundary commit's sha, so the next page pushed
  only that sha and silently lost every *other* commit still pending; a sibling that isn't its
  ancestor never gets pushed again. Repro: root `A`, `B(A)`, `S1(A)`, `S2(S1)`, `C(B)`, merge
  `M(C, S2)`, `limit=2` — page 1 emits `M, C`, pending is `[S2, B]`, the cursor was `S2` alone, and
  `B` never appears on any page.
- **The cursor is `"<start-sha>.<offset>"`** (`repo/commits.rs::Cursor`): every page re-walks from
  the same fixed start and skips `offset` filtered commits. Provably lossless — it is one walk cut
  at different points, so it can't diverge from an unpaginated walk. cgit's `ofs=` does the same.
- **`Cursor::MAX_OFFSET = 100_000`** bounds the walk a manipulated cursor can force (same rationale
  as #26/#28's scan budgets); exceeding it is `400 invalid_param`, like a malformed cursor.
- **The old bare-sha format is rejected outright, not accepted as a legacy alias** — the cursor is
  documented as opaque, and a fallback path would keep this exact bug reachable for anyone holding
  an old link.
- Cost went from O(1) to O(page index × `limit`) per page. Still strictly better than the O(repo
  size) any sort flag would force (#33), and self-limiting: a client paging `N` deep pays for `N`
  re-walks. Each page is still absorbed by the response cache.
- **Rejected alternative: a frontier cursor** (encoding the pending-list sha set at the boundary).
  It keeps O(page) cost, but libgit2's `seen` flag doesn't persist across requests — so a commit
  whose timestamp straddles a boundary under clock skew could be emitted twice — and the frontier
  needs a size cap, which reintroduces this very drop bug once a page touches enough branches.
- `commits::log` takes a `skip: usize`; `feed.rs` passes `0`. `CommitsPage`'s shape is unchanged, so
  the web needed nothing — `next_cursor` was always round-tripped as an opaque string.

## #38 raw patch and rawdiff: in-process git2, not exec; email exception on `/patch`

- **`git2::Diff::print`/`Email::from_diff` in-process, not `git format-patch`/`git diff` exec.** The
  #2/#12/#13 hybrid policy reserves exec for heavy streaming operations libgit2 has no equivalent
  for; that doesn't apply here. Exec'ing git would also introduce a **second diff engine** that
  disagrees with the first — git's rename detection and libgit2's `find_similar(None)` aren't
  guaranteed to agree, and git's `diff.indentHeuristic` has defaulted on since 2.14 while libgit2's
  `GIT_DIFF_INDENT_HEURISTIC` defaults off — so the structured JSON diff and its `.patch` link on
  the same page could show different file lists or hunk boundaries. Staying in-process also inherits
  `cached_response`'s cache, `ETag` and immutable `Cache-Control` for free.
  `repo/diff.rs::build_diff` is the single git2 diff chokepoint every entry point shares.
- **No line/file caps on `/rawdiff` or `/patch`**, unlike the structured diffs' `MAX_DIFF_FILES`
  (300) / `MAX_FILE_DIFF_LINES` (1000). Those caps bound a browser-rendered payload; a patch with
  hunks dropped is a *corrupt* patch that misapplies under `git apply`/`git am`. `/patch`'s
  commit-*count* axis is bounded instead (`MAX_PATCH_COMMITS = 100`) — by **rejecting** an oversized
  range with `400 invalid_param`, never truncating, since a silently shortened series applies
  cleanly and quietly corrupts history. If `/rawdiff` ever needs a bound, the next step is the
  exec+stream escape hatch, not a cap.
- **`from` means the opposite thing on `/rawdiff` vs `/patch`.** On `/diff`/`/rawdiff` it is the
  other side of a two-dot tree comparison (`git diff <from> <to>`); on `/patch` it is the
  **excluded** start of a commit range (`git format-patch <from>..<to>`). This matches git's own two
  conventions, but it is the most confusable part of the API surface and is spelled out with a
  worked example in `api/README.md`. `/patch` also diffs every commit against its own first parent,
  including merges — `git format-patch` skips merges, but every other axgit diff is first-parent.
- **`/patch` is a deliberate, narrow exception to "email addresses are never exposed"** (#8).
  `git am` cannot preserve authorship without a real `From: Name <email>` header, so redacting it
  would produce a patch that applies but records the wrong author. Nothing new is actually
  disclosed — the same data is already served unauthenticated inside every packfile over Smart HTTP
  (#13) — what changes is harvesting economics, mitigated with `X-Robots-Tag: noindex, nofollow` and
  `Content-Disposition: inline`. Every other response keeps `email_hash` only.
- `sanitize_component` lives in `handlers/mod.rs` so archive downloads and `/patch` filenames share
  one ASCII-safe implementation.

## #39 Compare page, side-by-side view, and Shiki highlighting in diffs

- **`/{repo}/diff` is a real route, not a query param on the commit page.** A comparison between two
  arbitrary revisions isn't "a commit", and reusing the commit shell would make every commit-page
  assumption (one `sha`, one `parent`) conditional. cgit's two-revision shape (`cmd=diff&id=&id2=`)
  remaps here; `id`/`id2` never collide with this page's own `from`/`to` keys.
- **The `(diff)` link on a parent row needs its own accessible name.** With a `Diff` tab and one
  `(diff)` link per parent on the same page, overlapping accessible names are an accessibility-tree
  ambiguity and a guaranteed Playwright strict-mode failure. Each is `Diff against parent <sha>` via
  `aria-label` — the same treatment the commit page's `Tree` link gets against the `Tree` tab.
- **Side-by-side pairing reuses cgit's `ui-ssdiff.c` algorithm**: consecutive deletions and
  additions are collected separately and paired index-for-index once the run ends, rather than
  assuming a hunk alternates `-`/`+` one-for-one — otherwise a 3-deletion/1-addition block silently
  drops two deleted lines.
- **Intra-line highlighting is hand-rolled (`lib/diff/intraline.ts`), not a dependency.** Three
  tiers — common prefix/suffix trim, a budget-capped (250,000 char-product) word-level LCS, then a
  whole-middle-changed fallback — at a fraction of `diff`/`diff-match-patch`'s size. Shipping a diff
  library would undercut #19's reason for choosing Shiki's JS-regex engine.
- **Shiki reconstructs each file's shown lines per side, not per line.** `context + deletion` and
  `context + addition` are each joined and tokenized once (`lib/diff/file-highlights.ts`), then
  mapped back to their `Line` object by identity, so a multi-line construct is far less likely to be
  mis-highlighted than tokenizing lines in isolation. A hunk boundary can still land mid-construct;
  that residual inaccuracy is accepted rather than fetching full blobs for both sides of every file.
- **Shiki's foreground colour and the intra-line background are composed, not nested.**
  `lib/diff/merge-tokens.ts` slices each segmentation at the other's boundaries so one pass of
  `<span>`s carries both.
- **Added/deleted files always render unified**, regardless of the chosen view — one side is empty
  either way, and cgit has no split concept for these either.

## #40 "Compare" entry point on the refs page: two independent directions

- **`getRepo(repo)` is fetched in its own effect**, parallel to and independent of `RefsView`'s
  `getRefs`, purely to read `default_branch` (`RefsInfo` has no such field). Decoration, not required
  data: a failure never touches the page's loading/error state, it just leaves the Compare cells
  empty — the same rule as `useCommitRefs` (#34).
- **Branches and tags compare in opposite directions.** Branches: `from={default_branch}`,
  `to={branch}` — "what does this branch have that the default doesn't". Tags: `from={tag}`,
  `to={default_branch}` — "what's landed since this tag". A tag almost always points at an ancestor
  of the default branch, so the branch direction would make most tag comparisons empty or backwards.
- The default branch's own row gets an em dash, not a self-comparison link.
- **Each link's accessible name is `Compare {from} with {to}` via `aria-label`** — a table of
  identically-named "Compare" links is an accessibility-tree ambiguity and a Playwright strict-mode
  failure (same reasoning as #39).

## #41 Idle compare page prefills `to` from the default branch

- **Prefills `to`, not `from`** — mirrors the api's own default direction (`to` defaults to `HEAD`,
  `from` to `to`'s first parent), making that default visible rather than inventing a new one.
- **No auto-fetch.** `/{repo}/diff` with no query params is still idle; only the input's value and
  the idle copy change. `useDefaultBranch` is enabled only while idle, so a page that already has a
  comparison makes no extra request.
- **`lib/default-branch.ts::useDefaultBranch`** is the shared decoration-fetch hook (`null` while
  loading, on failure, for an empty repository, or when disabled; failures silent), following
  `useCommitRefs`'s precedent.
- **The prefill is imperative (a ref), not `defaultValue`/`key`.** The form is deliberately
  uncontrolled (plain `method="get"`, works without JS), and React ignores a changed `defaultValue`
  on re-render. A `key` remount would show it but discard anything typed while the fetch was in
  flight and steal focus. Safe against `@base-ui/react`'s `Input`, which renders a native
  `<input defaultValue>` with no React state for the value and forwards the ref onto the element.
  The effect assigns only while the field is still `""`, so it never clobbers manual input.

## #42 Commit log pages are immutably cached when the request pins the walk start

- **Rule**: `cursor.is_some() || ref == Some(resolved_start_sha)`, computed in
  `handlers/commits.rs::list_commits`. `cursor` is unconditional because the token already encodes a
  full-sha walk start (#37) and `ref` is ignored whenever a cursor is present. A bare full-sha `ref`
  mirrors `search.rs`/`stats.rs`. The empty-repository page (no `ref`, no `cursor`) stays mutable.
- **Why cursor-always is sound.** `commits::log` is a pure function of the object graph reachable
  from `start` — the pending list's order depends only on commit objects, not refs/HEAD/packfile
  layout (#37) — and `touches_path` only compares tree-entry oids. So the page is
  `f(start, offset, path, limit)`, and `path`/`limit` are already in the cache key. A cursor pointing
  at a GC'd commit behaves exactly like `/commits/{sha}` does today: a fresh compute 404s, an
  already-cached immutable entry keeps serving until TTL/eviction.
- **Blast radius is bounded server-side**: immutable entries go through the same moka `insert`, so a
  stale body only survives one `AXGIT_CACHE_RESPONSE_TTL` window plus the byte cap. Only the
  browser's copy is pinned for a year. Behavior change: a cursor page's body can no longer change
  from a push, so it survives to TTL/LRU instead of being evicted by the validator — page 1 still
  invalidates on HEAD move and produces a different `next_cursor`, so an active repo's paging chain
  self-refreshes from the tip.
- `api/README.md`'s generic caching bullet now reads "pins the resource to a full sha — whether a path
  segment or a query parameter", matching what `/diff`, `/search` and `/stats` already did.

## #43 Diff stat-only mode (`view=stat`), with a `stat=1` fast path on `GET /diff`

- **The api half wasn't optional.** Client-side-only stat rendering works for the commit page, whose
  `/commits/{sha}` response already carries an uncapped `diffstat` — but not for the compare page,
  where stat mode exists precisely because a diff is too big, and `GET /diff` would still ship up to
  300 files × 1000 lines of hunks the page throws away. `?stat=1` skips `render_files` entirely and
  computes only the already-uncapped `diffstat`.
- **`rev_diff_stat` is a separate function, not a boolean branch inside `rev_diff`** — "hunks are
  never rendered in this mode" is then a property of the call graph rather than a runtime branch.
  `stat` is part of the cache key and factors into immutability exactly like `context`/`ignorews`,
  since it doesn't change what `from`/`to` resolve to.
- **`/rawdiff`'s query struct was split off** (`RawDiffQuery`) rather than adding `stat` to the
  struct both handlers shared: `stat` is meaningless on a plain-text patch (#38 already ruled out
  truncating one), and leaving it shared would put a meaningless parameter in that endpoint's spec.
- **The commit page and compare page fetch differently on purpose.** `CommitView` skips the
  `getCommitDiff` request outright in stat mode — it already has the numbers. `DiffView` fetches
  `stat=1` *without* `context`/`ignorews`, since neither affects `diffstat` and dropping them
  normalizes every stat-only comparison for a given `from`/`to`/`path` onto one cache entry.
- **`view` gained a third value (`"stat"`)** rather than a parallel boolean —
  `DiffOptionsBar`'s pill row, the URL round-trip and `diffOptionsQuery` all already treat `view` as
  the one display-mode axis. `DiffFileList`/`DiffFile` narrow to
  `HunkViewMode = Exclude<DiffViewMode, "stat">` so the type system states that stat mode never
  renders them, instead of a runtime guard.
- **Each stat row links to that file's own single-file diff** (`path=` + `view=unified`), not a
  `#diff-N` anchor — in stat mode there is no file list to jump to. `DiffStatTable` takes an optional
  `hrefFor` (default: the existing anchor). This is why `commitHref` gained a `path` param and why
  both views support `?path=` at all. A path-filtered diff shows a "Showing only `{path}` — Show all
  files" line; without it the only way back would be the browser's back button.

## #44 Log tab message expansion (`msg=1`), cgit's `showmsg=1` parity

- **`CommitInfo` gained an optional `body`, keyed off `msg=` on `GET /commits`** (`parse_flag`,
  default off). The default log payload is unchanged, and so is `/search?type=message`, which reuses
  `CommitInfo` through `commit_info`; only the log walk uses `commit_info_with_body`, so search
  never has to think about the field.
- **`body` is an omitted key, not a `null` value, when there is nothing to show** — deliberately
  unlike every other optional field in `CommitInfo`/`CommitDetail`, which are always-present
  nullable keys. A caller that didn't ask for `msg=1` shouldn't see `body: null` on every entry, and
  one that did can treat "key present" as "there's something to render".
- **`msg` is folded into the cache key's `params`**, like `stat` on `GET /diff` (#43), or a `msg=1`
  response would alias onto the default one. It does **not** affect immutability (#42) — for a fixed
  walk start it only picks which fields come back, not which commits.
- **The graph column needed a second row shape.** `CommitLog`'s `h-12` row is a fixed-height grid
  (#33) an arbitrary-length message can't fit, and hiding the graph for expanded rows would break
  the line for every commit. `CommitGraphSpacer` is an absolutely positioned SVG holding only
  straight verticals for `row.through ∪ row.out` — no node, no diagonals, since those belong to the
  row above — stretched by CSS rather than a fixed `viewBox`.
- Each commit is a `Fragment` wrapping its row plus an optional message row, so "does this commit
  have a body row" stays a single `&&` next to the row it belongs to.
- **The toggle is a URL-only link**, not component state — the same "display option lives in the URL"
  rule as `DiffOptionsBar` and `stat=1`. It preserves `ref`/`path`/`cursor`, and "Older →" carries
  `msg` forward.

## #45 Git notes on the commit page (cgit's `format_display_notes()` parity)

Scope is the commit page only; the log's `msg=1` rows stay note-free.

- **A note is mutable state on an otherwise immutable resource.** A `git notes` message is not part
  of the commit object — it lives on `refs/notes/commits` and can change without the commit sha
  changing — so `GET /commits/{sha}`'s immutability premise breaks the moment a note exists.
  `get_commit`'s flag is now `sha == detail.sha && detail.note.is_none()`: a note-less commit behaves
  exactly as before, a noted one falls back to `ETag` + `no-cache` and always revalidates against the
  HEAD+agefile validator (which does move on a notes push, since post-receive touches the agefile for
  any push to the repo).
  - Rejected: a dedicated `GET /commits/{sha}/notes` endpoint — it would cost the commit page a
    second parallel request for what is usually nothing, and the `msg=1`/`stat=1` precedent is
    opt-in query params, not sibling routes, for exactly this "small, commonly-absent field" shape.
    Also rejected: a `notes=1` opt-in param — the web always wants the note when present, so it
    would be requested unconditionally, adding a query string with no choice behind it.
  - Accepted limitation: a browser that cached a commit page **before** a note was added keeps the
    note-less copy for up to a year. Inherent to immutable caching for full-sha resources (#42),
    not new here. Server-side, the moka entry refreshes within one TTL window.
- **Only the default notes ref is read** — `find_note(None, oid)` (`core.notesRef`, else
  `refs/notes/commits`). cgit also honors `notes.displayRef`/`GIT_NOTES_DISPLAY_REF` and
  concatenates multiple refs; axgit reads one. Any lookup failure — no notes ref, no note, non-UTF-8,
  all-whitespace — collapses to `None`, never an error: a repository with no notes ref must not turn
  a working commit page into a 500.
- **`note` is a required-but-nullable field**, not an omitted key — unlike `CommitInfo::body` (#44),
  it is never opt-in, so every response carries the key.
- **Rendered as its own block**, not folded into the message `<pre>`: a left accent border under a
  "Notes" heading, below the message and above the diffstat, linkified through the same `linkify()`
  (cgit's `format_display_notes()` is plain text too). A note is not something the author wrote when
  making the commit, so it stays structurally distinct — and a note never makes an absent message
  block appear.
- Fixtures: `scripts/make-fixtures.sh` attaches one note to `git-compose.git` and pushes
  `refs/notes/commits`; notes live on their own ref, so no fixture commit sha changed.

## #46 Search API: `type=author|committer|range` (cgit `qt=` parity)

Same endpoint and response envelope as #26 — one type selector, not three more routes.

- **`author`/`committer` match the signature *name* only, never the email.** Matching the email
  would turn the search box into a confirm/deny oracle for a specific address, which the hash-only
  design (#8) exists to prevent. A deliberate difference from cgit's `--author=`, which matches
  `Name <email>`, and documented in `api/README.md` so it isn't mistaken for a bug.
- **`range` treats `q` as a rev-list expression, not a text filter** — `q` alone names everything to
  include, the way `git log <range>` ignores the checked-out branch. `ref` still resolves the
  response's `sha` (keeping the empty-repository carve-out) but plays no part in the walk.
  - **A token starting with `-` is `400 invalid_param`.** Nothing here is ever exec'd, so this is
    not an injection guard — it exists so a flag-shaped token (`--all`, `-n5`) gets a clear 400
    instead of libgit2 trying to `revparse` it and failing with a confusing `ref_not_found`.
  - **`A..B`/`A...B`/`^X` are parsed by hand (`repo/search.rs::walk_range`), not via
    `git2::Revwalk::push_range`**, which explicitly rejects `A...B` (`revwalk.c:253`, "symmetric
    differences not implemented in revwalk"). Hand-parsing both gives one code path, one error type
    (`resolve_commit`'s `RefNotFound`, so an unresolvable token is 404, never 500), and one
    consistent "empty side means `HEAD`" rule. `A...B` uses `merge_bases` (plural — a criss-cross
    history can have more than one) with every base hidden before both sides are pushed.
  - Every revision goes through the same `resolve::resolve_commit` as everything else — no parallel
    resolution logic.
- **`range` never gets immutable caching, even with a full-sha `ref`.** Immutability elsewhere means
  "the request's `ref` is the resolved commit's own sha", a sound proxy only while the API's
  resolution of `ref` is the sole input. `range` breaks it — `main~5..main` moves whenever `main`
  does, independently of `ref`. So `immutable = kind != SearchKind::Range && ref == sha`.
- `MAX_SCANNED_COMMITS` (10,000) and `limit` apply exactly as `message` already used them.

## #47 `/{repo}/search` page: surface `type=author|committer|range`

- **No new component.** All four commit-producing types return `SearchResults.commits` in the same
  `CommitInfo` shape, so the `results.type === "message"` branch became a `COMMIT_KINDS` membership
  test rather than three more `||` arms.
- **`TYPE_OPTIONS` is the single source of truth for valid `type` values.** `isSearchKind` derives
  `SEARCH_KINDS` from it rather than hard-coding its own `===` chain, which would silently drift the
  moment one list was updated and the other wasn't.
- **A one-line hint under the form for `type=range`** — every other type is a plain text match, but a
  rev-list expression is a different enough input shape that the box alone gives no clue. Shown from
  the URL-derived state, not live `<select>` input, keeping the page free of controlled form state
  (#27).

## #48 Per-row quick links on the repository index and the tree listing

- **A shared `IconLink` component, not a `RowActions` component.** Both tables need the same
  accessibility contract — an icon-only link whose name comes entirely from `aria-label`, icon
  `aria-hidden` — so that is shared (`web/src/components/IconLink.tsx`, top-level, since the index
  isn't a repo page). What is *not* shared is which actions apply to which row: the tree listing
  varies by entry kind (submodules get nothing, directories get Log, blobs/symlinks get Log + Raw +
  Blame) while the index has no kind at all. A component owning "the action set for a row" would have
  to parameterize around a concept only one caller has.
- **Directory rows get a Log link too** — `?path=` on `/log` matches a directory prefix, not just a
  file, so "this directory's history" is a real link; cgit does the same.
- **New `aria-label`s make non-exact `getByRole("link", { name })` lookups ambiguous** — once a row
  has action links, `"main.rs"` is a substring of `"Blame for main.rs"`. Affected lookups use
  `exact: true`.
- Skeletons needed no change: they model row *height* as plain bars, and a `w-px` icon column adds
  none.

## #49 Symlink targets in the tree listing (cgit `name -> target` parity)

- **Served verbatim, resolved client-side.** `TreeEntryInfo.target` is the raw stored path, relative
  to the entry's own directory, with no server-side normalization — a `../` prefix reaches the caller
  intact. Resolving server-side would mean deciding what an escaping target becomes in a field whose
  contract is "this is what git stores", and would lose the distinction between a relative link and
  one written from the root.
- **The existing `odb.read_header` call is the size gate** — already in the entry loop for a blob's
  `size`, and a stat rather than a load. Widening it to symlinks bounds the target read at
  `SYMLINK_TARGET_LIMIT` (4096, PATH_MAX) for free. **Over the cap reports `null` rather than
  truncating**: half a path is a *wrong* target, not a shorter one, and a caller that linked it would
  point somewhere real but incorrect. Non-UTF-8 collapses to `null` too — one nullable field, three
  benign causes.
- **`size` stays blob-only** — for a symlink it is just `target.len()` and reads as noise in the Size
  column.
- **`resolveRepoPath` is reused, not reimplemented** (`lib/markdown-url.ts`, from #21). The base is
  the **listed directory**, not the entry's own path.
- **The raw target is displayed, the normalized one is linked** (as cgit does); a `null` resolution
  renders as plain text. A target naming a *directory* still gets a blob href — the kind isn't
  knowable from a listing that only saw the link, and the blob endpoint 404s cleanly rather than
  guessing.
- Fixture targets are chosen not to collide with any row name, or they re-create #48's strict-mode
  ambiguity.

## #50 `Others (N)` on the stats page, instead of silently truncating authors

- **A separate `others` object, not a synthetic entry appended to `authors`.** `AuthorStats` requires
  a `CommitAuthor` with a `name` and `email_hash`; an aggregate has neither, a sentinel would break
  `AuthorAvatar`'s identicon seed, and `authors.length` would become `limit + 1`, which the
  "Showing top N of M" hint compares against `author_count`. Keeping it separate meant that hint and
  the chart needed no change at all.
- **`count` is carried explicitly** even though it equals `author_count - authors.len()` — that
  arithmetic is only correct if the caller already knows `authors` is exactly the capped list.
- **The aggregation is free** — each `AuthorStats` already carries its bucket vector, so
  `split_off(limit)` leaves a tail needing only an elementwise sum. No second revwalk.
- **`others != null` is the precise "`limit` cut authors" signal.** `truncated` deliberately stays the
  union of both causes (matching search), but `truncated && others == null` now isolates the scan
  budget.
- **The footer still reads the top-level `buckets` rather than summing rows.** They agree column for
  column, but that is a property of the response to assert in tests, not an invariant to re-derive in
  the view — a comment says so, since it is exactly what a later reader would "simplify".

## #51 Tag detail endpoint + page, and per-tag archive downloads

- **`TagRef.target` keeps its existing meaning (the peeled commit sha) on the new endpoint too.**
  Redefining it as the one-level dereference on a sibling endpoint would make one word mean two
  things across the same resource family, and `lib/commit-refs.ts` already keys ref badges off the
  peeled meaning. The one-level dereference got its own name, `object: { sha, type }` — cgit's own
  vocabulary.
- **`target` is `null`, not an error, when the tag never reaches a commit** (a tag on a tree or
  blob) — deliberately the same condition under which an archive download is unavailable, so one
  field answers both questions.
- **Lightweight tags resolve `200`, not `404`** — `GET /refs` lists them, so a 404 would make every
  lightweight tag's link on the refs page dead. `tag_object`/`message`/`tagger`/`tagged_at` are
  `null` instead, which is the precise "this is lightweight" signal.
- **Never immutably cached.** The URL names a tag *ref*, not a sha, and a tag can be force-moved onto
  a different object without its name changing — immutability is a property of the address, and this
  address is mutable by construction. (Pre-existing limitation, not new: the validator is HEAD sha +
  agefile mtime, so a tag move alone can stay invisible for one TTL window, as for `GET /refs`.)
- **`repo::tag` is its own module, not folded into `refs.rs`** — `TagDetail` isn't a superset of
  `TagRef` the way `CommitDetail` is of `CommitInfo`. The tagger reuses `signature_info` so the
  no-raw-email invariant holds.
- **Web: a drill-down from the Refs page, not a `RepoNav` tab.** Tabs carry a single fixed path
  segment; a tag name is unbounded and unknown at build time. `shellFor`/`shell_for`'s `tag` arm uses
  the same "at least one segment" rule as `blob`/`blame`, since a tag name may contain `/`.
- **Only `object.type === "commit"` gets a link on the tag page** — see #53, which later closed the
  by-oid gap this left open.
- **The download column is tags-only, not also on branches**, matching cgit's `print_tag_downloads()`.
  A branch archive's filename names a moving target; a tag's is reproducible. `archiveUrl` itself
  accepts any ref — this is a scope choice, not a capability gap.
- **Download links use visible text + an overriding `aria-label`** (`Download {tag} as {format}`),
  not `IconLink` (#48). `IconLink` fits when a distinct icon carries the row's information; here the
  format name *is* the information, and two identical download glyphs would be indistinguishable.
- **`RefBadges.tsx` is deliberately unchanged** — its tag badge links to `logHref` ("commits at this
  ref"), a different intent from the tag page ("this tag itself").

## #52 Remote branches on the refs page, built despite finding no real use case

Built on request as future-proofing, after an investigation found no repository that would use it.

- **Nothing in axgit or the git-compose stack populates `refs/remotes/*`.** Every repository arrives
  via SSH push, never `git remote add` + `fetch`. Worth recording: `git clone --mirror` lands
  upstream branches in `refs/heads/*` via its `+refs/*:refs/*` refspec, so a mirrored repository was
  *already* fully visible through the local-branch path — `refs/remotes/*` only appears from a
  hand-configured remote with a custom fetch refspec against a bare repo. `api/README.md` says so
  plainly, so a future reader doesn't wonder why the field is always empty.
- **A separate `RefsInfo.remote_branches`, not merged into `branches`** — merging would silently
  change what `RepoSummary.branch_count` counts. It reuses `BranchRef`, since git2 exposes no way to
  split a remote branch's name from its remote (`"origin/main"` is the whole shorthand).
- **A remote's own symbolic `HEAD` (e.g. `origin/HEAD`) is skipped.** libgit2 selects purely on the
  `refs/remotes/` prefix and peels it to a commit just fine, so the "unresolvable tip" guard wouldn't
  catch it — it would show as a redundant alias row.
- **Web gets Log and Compare links, not Tree.** `?ref=`-driven endpoints already resolve
  `origin/main`-shaped names via `revparse_single`'s DWIM rule. Tree/blob/blame differ: their
  `{ref}/{path...}` route depends on `resolve.rs::ref_shorthands()`, which only collects
  `refs/heads/`/`refs/tags/`. Extending it raises a real disambiguation question (a local branch
  `origin` beside a remote branch `origin/main` under longest-match) not worth resolving for a
  feature with no confirmed user.
- **The section renders nothing at all when empty**, unlike Branches/Tags' always-present "No
  branches." — a permanent "No remote branches." on every page would be pure noise.
- Test fixtures use `git update-ref refs/remotes/{name} {target}` directly, matching the finding that
  this is a hand-configured shape rather than something to simulate a second repository for.

## #53 Object links for non-commit refs (`GET /objects/{oid}`, cgit's `cgit_object_link()` parity)

- **`TagRef` gains `object: { sha, type }`, and `target` becomes nullable.** `GET /refs` used to fall
  back to the *peeled object id* for a tag that never reaches a commit, writing a non-commit oid into
  the field a commit sha goes into, under a column headed "Commit". That fallback is deleted;
  `object` is the same one-level dereference `GET /tags/{name}` reports (#51's "one word, one
  meaning"), and `target` keeps its nullable meaning. `tag::dereference` is shared by both endpoints
  so they structurally cannot drift apart again.
- **`GET /objects/{oid}` requires a full 40-character hex id; abbreviations are `400 invalid_param`.**
  Every link axgit emits carries a full oid, and requiring one is what lets this endpoint be
  **unconditionally immutable-cached** — the address *is* the content, so there is no validator to
  check. `repo::object::parse_full_oid` rejects on length or non-hex bytes before touching the
  repository or the cache.
  - **One envelope, not a `oneOf` union** (`ObjectDetail { sha, type, tree, blob, tag }`, only the
    `type`-matching payload non-null), keeping the document-wide "every key always present"
    invariant. A `commit`-typed object carries no payload — the web view links to
    `GET /commits/{sha}` rather than duplicating it.
  - **`TreeEntryInfo` gains `sha`** (additive on `GET /tree` too) so entries can link onward by oid;
    `repo::tree::entries_of` is shared between the ref+path and by-oid tree endpoints so they can't
    disagree on ordering or field shape.
  - **`repo::tag` has two parallel dereference helpers.** `dereference` takes a `git2::Reference` and
    uses `Reference::peel_to_commit()`; a tag reached by its own oid has no reference behind it, only
    a resolved `git2::Tag`, so `dereference_object` peels via `Object::peel`. Kept as two functions
    rather than one taking a `Reference`-or-`Object` enum threaded through otherwise identical code.
  - **`ApiError::ObjectNotFound` (`404 object_not_found`) is a new variant**, not a reuse of
    `RefNotFound`: a by-oid lookup involves no resolution at all, just an object-database miss, and
    conflating them would blur a distinction every other 404 here already draws.
- **`GET /objects/{oid}/raw`** is the by-oid analogue of `/raw/{ref}/{path...}`: blob bytes only,
  always immutable, always `nosniff`. With no filename behind an oid there is no extension to guess
  from, so it is always `text/plain; charset=utf-8` or `application/octet-stream` — never
  `mime_guess`.
- **Web: `/{repo}/object/{oid}`**, an exactly-3-segment shell arm (an oid never contains `/`), nav-
  highlighted as "Refs" like the tag page — both are drill-downs, not places a user browses to.
  - **Tree navigation inside the object page is oid-to-oid, with no path context** — there is no root
    commit behind a bare oid to build a breadcrumb from, so `ObjectView` doesn't fake one. A gitlink
    entry stays inert text; its sha names a commit in another repository.
  - **A blob's `CodeBlock` gets `path=""`** — no filename means no language guess, so it renders as
    plain text, the same fallback an unrecognized extension gets.
  - **A tag's non-commit dereference links onward through the object page too**, which is what lets a
    nested tag be followed one hop at a time.
  - **`Disallow: /*/object/` in `robots.txt`** — the object graph is walkable link-by-link, cheap per
    request but not worth handing a crawler, matching the scan-budgeted-endpoint rule.

## #54 Archive format coverage (`tar.bz2`/`tar.xz`/`tar.zst`, cgit's snapshot format parity)

- **In-process streaming encoders (`async-compression`), not external compressor binaries, and not
  git's `tar.<fmt>.command` config.** cgit forks `bzip2 -c`/`xz -c`/`zstd -c`; matching that would
  mean adding those packages to the runtime image and assuming they exist on every dev/CI host.
  `git archive --format=tar` output is wrapped in an `async_compression` encoder and streamed the
  way `tar.gz`/`zip` already were, so the reaper/`kill_on_drop` machinery needs no change and #12's
  "exec only ever sees a resolved sha" invariant is untouched — the encoder sits *after* exec.
  - `xz`'s backend is pinned via a direct `liblzma = { features = ["static"] }` dependency.
    Without it, `liblzma-sys` pkg-config-probes for a system liblzma and links it dynamically when
    one exists (e.g. Homebrew's `xz`), making the linked xz version depend on the build host — the
    exact class of problem `libgit2-sys`'s vendored build already avoids.
- **Plain `tar` and cgit's `tar.lz` are deliberately not offered.** An uncompressed multi-megabyte
  HTTP download has no use distinct from `tar.gz`; `tar.lz` has no maintained Rust encoder, so
  matching it would reintroduce the external-binary dependency this decision avoids. `main.tar` stays
  `400 invalid_param`, pinned by a regression test.
- **A `FORMATS` table replaces the three-armed `format_arg`/`content_type`/`extension` match** —
  adding a format is a one-line entry rather than three match arms that could drift.
  `parse_archive_target` matches any suffix in the table, since none of the five is a suffix of
  another.
- **Compression levels are pinned to cgit's CLI defaults** (`bzip2 -9`, xz preset `6`, `zstd -3`),
  not `async-compression`'s `Level::Default` — which is bzip2 `6` and xz preset `5`, so leaving it
  would quietly serve weaker compression than cgit at the same format.
- **A bounded `Arc<Semaphore>` (`AppState::archive_encoder_limit`, 4 permits) gates only the three
  encoder formats**, acquired *before* `git archive` is spawned so an over-capacity request waits
  rather than spawning a process it isn't ready to read from. Archives are never response-cached, so
  every request builds a fresh encoder and an xz preset-6 encoder alone holds ~90 MiB; cgit's
  identical exposure was masked by each request being a separate forked process. `tar.gz`/`zip`
  never touch the semaphore. The permit is held by a `Guarded<R>: AsyncRead` wrapper for the response
  body's lifetime, released on drop.
- **`Content-Type` prefers a registered media type**: `application/zstd` (RFC 8878) for zstd;
  `bzip2`/`xz` have none, so the de facto `application/x-bzip2`/`application/x-xz` (as cgit uses).
- **ETag stays repo-scoped, not per-format** — HTTP caches key on the full URL, so `main.tar.gz` and
  `main.tar.zst` never collide. Stated explicitly so nobody "fixes" a perceived gap by folding the
  format into the validator.
- **A mid-stream `git archive` failure yields a well-formed-but-truncated file for the three encoder
  formats**, since the encoder still finalizes its container — unlike `tar.gz`, whose truncation
  produces an invalid gzip stream a decompressor rejects outright. Documented in `api/README.md`
  rather than treated as a defect; fixing it would mean buffering the whole archive before sending.
- **Tests decompress with the same crate the handler encodes with**, not a system binary — macOS's
  bsdtar doesn't reliably support zstd and GNU tar shells out for `-j`/`-J`/`--zstd`. Magic bytes are
  checked separately, which a matching decoder alone wouldn't catch if the container were wrong.
- **Web**: `ARCHIVE_FORMATS` in `lib/api/repos.ts` is the single list all three download call sites
  render from.

## #55 Archive download links on the commit page

- **The commit's own sha is the ref, not the URL's `{sha}` param** — `archiveUrl(repo, detail.sha,
  format)`, the resolved full 40-character id, not whatever the visitor typed. That is also what
  keeps the request on `handlers/archive.rs`'s `immutable = refname == sha` path.
- **This inverts #51's branch-exclusion rationale on purpose.** #51 kept downloads off branches
  because a branch archive names a moving target; a commit sha is the opposite — the most
  reproducible address `archiveUrl` can be given.
- **Rendered inline in the existing action row** with plain link text, not `RefsView`'s per-row
  `aria-label` variant: that override exists because the same text repeats down a column of tag rows,
  and this page renders one set.
- **No gating condition**, unlike `TagView`'s `canBrowse` — a tag can point past a commit at a
  tree/blob, but a commit always has a tree.
- **Diff display options are not threaded into the archive links** — they control how the diff
  renders; the archive is always the commit's full tree.
- **No shared component was extracted** for the four call sites; each wraps the links differently
  enough that a shared component would just grow option props, and the actual duplication (the format
  list) is already centralized.

## #56 Rename following in the commit log's path filter (`follow=1`, cgit's `enable-follow-links`)

Without this, `touches_path` treats a rename as "the old path stopped existing, the new path started
existing" — correct in isolation, but the log for a renamed file ends at the rename instead of
continuing under the old name, unlike `git log --follow` and `repo::blame` (#36).

- **`commits::log` takes a `LogParams` struct** (`path`/`skip`/`limit`/`include_body`/`follow`),
  the same shape `diff::DiffParams` uses.
- **The walk tracks a mutable `tracked: Option<PathBuf>`, reseeded from `path` on every call**, never
  resumed — consistent with the walk always restarting from `start` (#37). That is what keeps a
  `follow=1` cursor page lossless: every page re-derives the same rename chain from scratch, so
  nothing is page-boundary-dependent.
- **Rename detection only runs where a rename could plausibly be** — when the tracked path is present
  in the commit but absent from its first parent (`first_parent_lacks_path`). An ordinary add or
  modify never reaches the expensive full first-parent-tree diff this guards.
- **`diff::rename_source` diffs the *whole* first-parent tree, not a pathspec-restricted one** —
  unlike every other `build_diff` caller. A rename's old path can't be predicted, so there is nothing
  to restrict to; it reuses `find_similar`'s libgit2 defaults (the same 50% threshold as the diff
  endpoints).
- **Only `Delta::Renamed`, not `Delta::Copied`, is followed** — matching `repo::blame`'s
  whole-file-renames-only rule (#36), so the two history-following features agree. cgit's equivalent
  doesn't follow copies either.
- **`MAX_FOLLOW_RENAME_LOOKUPS = 100`** bounds how many full-tree diffs one walk can attempt (same
  budget rationale as `Cursor::MAX_OFFSET`, #26/#28). Past the cap the walk keeps filtering on
  whatever path it last tracked rather than erroring.
- **`follow` is silently a no-op without `path`**, in both the api and `CommitLog.tsx`. No `400` —
  cgit's `enable-follow-links` has nothing to be invalid about either.
- **`CommitInfo.renamed_from` is an omitted key when absent**, the convention `body` established
  (#44). `commit_info()` defaults it to `None`, so `/search`'s reuse is untouched.
- **`follow` joins `msg` in the cache key's `params`** — it doesn't affect immutability (a fixed
  start makes it a pure field/commit selector), but it changes the body, so it must not alias.
- **Web**: a `Follow renames`/`Stop following renames` URL-only toggle in the existing path-filter
  banner, preserving `ref`/`cursor`/`msg` (and vice versa).

## #57 Files/Lines changed columns on the commit log (`stat=1`)

cgit ships these as two independent flags; axgit merges them into one `stat=1` — there is no
per-repo display config here to keep them separate for, and splitting would double the cache-key
value space for no use case.

- **`diff::stat_counts` is a cheaper sibling of `diffstat`** — one `Diff::stats()` call instead of a
  `Patch::from_diff` per file. The log column needs three totals per row; the per-file loop would be
  wasted work multiplied by `limit`. Both share `commit_trees`/`build_diff`, so both stay
  first-parent by construction, merges included.
- **Restricted to the log's own `path` filter, using the snapshot `follow` introduced (#56).**
  `commits::log` clones `tracked` into `filter_path` at the top of each iteration, *before* a rename
  crossing can mutate it, and passes that snapshot to both `touches_path`/`rename_source` and
  `stat_counts`. Using the post-mutation `tracked` would make the renaming commit's stat count
  against its *old* name instead of the name it was filtered in under.
  - **This restriction is `path`-only, not rename-aware**, unlike the diff endpoints: a pathspec
    restricts the tree diff *before* `find_similar` runs, so on the renaming commit
    `path=<new name>&stat=1` sees a plain addition rather than a zero-change rename. Documented in
    `api/README.md` rather than special-cased — a real `git log --stat -- <path>` behaves the same way.
- **`CommitInfo.stat` is an omitted key when absent**, like `body` (#44) and `renamed_from` (#56);
  `commit_info()` defaults it to `None`, so `/search`'s reuse is unaffected.
- **`stat` joins `follow`/`msg` in the cache key's `params`** — irrelevant to immutability, but it
  changes the body, so it can't share an entry with the default response.
- **Web**: a `Show changes`/`Hide changes` URL-only toggle beside `Expand messages`, plus two
  right-aligned `Files`/`Lines` columns. Unlike `msg`/`follow`, `stat` doesn't depend on a `path`
  filter, so the toggle always renders once there are commits. The expanded-message row's `colSpan`
  is computed (`4 + (showStat ? 2 : 0)`), not hardcoded.

## #58 Hex dump view for binary blobs (cgit's `bin-blob` parity)

Web-only. The bytes come from the raw endpoint the blob/object pages already link, fetched
client-side rather than added to `BlobInfo`/`ObjectBlob` — `content: null` already means "fetch raw
instead", and a base64 copy would bloat every binary blob response by a third for no reader that
doesn't already have the page open.

- **16 bytes per row, not cgit's 32** — matches `xxd`/`hexdump -C` and keeps the row narrow enough to
  avoid horizontal scroll on a phone. Rendered immediately when `binary: true`, not behind a toggle.
- **Two independent boundaries, each reusing an existing number.** The *fetch* is gated on
  `!blob.too_large` (the api's 1 MiB `BLOB_CONTENT_LIMIT`), so an over-1-MiB binary never triggers a
  raw fetch at all. The *render* is separately capped at `HEX_DUMP_LIMIT = 64 KiB` (4096 rows) with a
  visible "Showing the first 64.0 KiB of N — view raw" note — a sub-1-MiB binary is still tens of
  thousands of DOM rows at 16 bytes/row.
- **`lib/format/hex.ts::hexRows` is a pure layout function** (the `commit-graph.ts`/`markdown-url.ts`
  precedent), unit-tested on its own. It truncates first, then emits per-row offset, per-byte hex
  strings and a parallel ASCII string; group spacing and short-final-row padding stay in the
  component, the same split `CommitGraph` uses.
- **`fetchRawBytes` lives beside `rawUrl`, not in `client.ts`** — `apiFetch` is JSON-only by
  construction and keeps `API_BASE` module-private, so `fetchRawBytes` takes the full URL a
  `rawUrl`/`objectRawUrl` call already produced. Same `ApiError` contract, so the caller needs no
  special-casing.
- **A failed fetch renders the same "Binary file not shown — view raw" notice** the pages used to
  show unconditionally, so a network error degrades to the old behavior rather than a blank panel.
  Wired into both `BlobView` and `ObjectView`, which were identical dead ends.
- Fixtures gained a small binary file with a NUL-containing pattern — extension alone doesn't trigger
  `Blob::is_binary()`, so this path was otherwise unreachable in a local run.

## #59 Stats API: `path=` filter

- **Reuses `repo/commits.rs::touches_path` verbatim, promoted to `pub(crate)`.** It is the same
  question the commit log's `path` filter already answers, and two call sites drifting into slightly
  different merge-commit approximations would be worse than one function with two callers.
- **No `follow` support.** cgit's stats page doesn't track renames either, and `/commits`' `follow=1`
  machinery (#56) pays for a full first-parent tree diff on every rename-shaped commit — a cost the
  scan-budget family was built to bound, not add to. Cheap to add later if a real need appears.
- **The filter is applied after the bucket-window check, not before** — a commit that is simply too
  old never pays for the per-parent tree lookup. `MAX_SCANNED_COMMITS` counts commits the revwalk
  visits, not ones that pass the filter, so `truncated`'s meaning is unchanged.
- **A path that never existed returns `200` with all-zero buckets, not a `404`** — the same carve-out
  `/commits?path=` documents.

## #60 `/{repo}/stats` page: surface the `path` filter

- **The entry point is a `Stats` quick link on each tree row, not the nav tab.** cgit's stats tab
  carries the current `ctx.qry.vpath`, but axgit's tab bar is static HTML prerendered under a
  placeholder param (#17) and filled in by `window.__axgit.fillRepoShell`, which has no notion of
  "the tree page's current path". Extending that for one tab on one page would be a lot of machinery
  for what #48 already generalizes: a per-row action link built from data the row already has.
  Present on file and directory rows, absent on submodule rows.
- **`StatsView` resolves `path` with the same prop-overrides-`location` pattern as every other
  param**, carries it through every `statsHref` the period switcher builds (so switching period
  doesn't drop the filter), and renders a `Filtered by path … — clear filter` banner modeled on
  `CommitLog`'s, minus "Follow renames" (#59 has no `follow`).

## #61 Atom feed gains `ref`/`path`/`all`/`limit`

- **`?ref=`, never cgit's `h=`** — #18 settled ref selection for the whole API, and #35 treats `h=`
  as a redirect-only legacy alias.
- **`all=1`'s scope is `refs/heads/*` + `refs/tags/*` only**, matching the ref-shorthand resolver's
  definition of "a ref axgit can resolve". Not `refs/remotes/*` (#52 found nothing that populates
  it); widening later is additive, not breaking.
- **The multi-tip walk uses `Revwalk::push_glob` on the two globs**, not a manual `references()`
  loop: libgit2 peels an annotated tag to its target and silently skips any ref that doesn't peel to
  a commit, so no extra filtering is needed. Two explicit globs — not `refs/*` — keep `refs/remotes`
  and `refs/notes` out by construction rather than by an exclusion check.
- **`Sort::TIME`, but only on the multi-tip walk.** The single-tip `log()` stays unsorted; with one
  tip that is the closest match to `git log` and changing it would desync the feed from the Log tab.
  With several unrelated tips, the default DFS drains one tip's ancestry before touching the next —
  a stale tag pushed first would starve the walk and freeze `<updated>` forever. Deliberately *not*
  `Sort::TOPOLOGICAL | Sort::TIME` (which `format_patch` uses to keep a parent after its child):
  that reintroduces the branch grouping the feed is avoiding. Caveat: libgit2 sorts on **committer**
  date while `<updated>` reports **author** date, so a rebased history can still be non-monotonic —
  accepted, since readers sort by `<updated>` themselves and cgit has the same property.
- **`repo/commits.rs::log()` split into itself plus a private `collect()`** over an already-pushed
  `Revwalk`; `log_all_refs()` builds the multi-glob walk and returns `Vec<CommitInfo>` with no
  cursor. Deliberately *not* a `LogStart { Oid, AllRefs }` enum threaded through `log()` — a
  multi-tip walk has no single start to encode a cursor from, and the cursor-less return type makes
  that a fact of the type system rather than a runtime rejection.
- **`ref` is ignored when `all=1`**, the way `cursor` already makes the commit log ignore it —
  computed once as `effective_ref` before the cache key, canonical query and walk all read it, so
  `?all=1&ref=nope` never resolves `nope` and shares one cache entry with `?all=1`. A `400` was
  rejected: a bookmarked feed with a since-deleted `ref` would poll a permanent error forever.
- **No `follow` support**, the same call #59 made for stats.
- **`<id>`/`rel="self"` carry a canonical query string** built from the *parsed* params, not echoed
  from the request, so `?path=/src/`, `?path=src` and `?limit=20&path=src` resolve to one feed
  identity (RFC 4287 §4.2.6). Fixed param order, defaults omitted — so the all-defaults feed's `<id>`
  is byte-identical to the pre-#61 one. Entry `<id>`s stay `urn:sha1:{sha}`: cross-feed commit dedup
  is a different problem from a feed's own identity.
- **No immutable caching, even when `ref` resolves to a full sha.** The feed body's `<subtitle>`
  embeds `info.description`, read live from the repository config — mutable state on an otherwise
  sha-addressed resource, the same problem #45 solved for git notes. A #45-shaped
  `&& description.is_none()` would work mechanically but only fires for undescribed repositories, and
  a sha-pinned feed can never gain an entry, so no reader would subscribe to one.
- **`handlers::parse_limit` gained a `default` parameter** so the feed can use 20 without duplicating
  the "1–100, never clamped, `invalid_param` on failure" logic.

## #62 `/{repo}/log` and the summary page surface the feed parameters

- **The link goes in `CommitLog`'s action row, not the path-filter banner** — the banner only renders
  when a path filter is set, so a `?ref=dev` log would get no feed link. Deliberately forwards only
  `ref`/`path`: `msg`/`stat`/`follow`/`cursor` have no feed analogue, and the log's page size is a
  different concept from the feed's item count.
- **`RepoSummary` gains an `All refs` link** beside the plain Atom link — "every branch and tag" is a
  repository-level concept, not tied to a particular log view, and without it there is no UI path to
  `all=1` at all.
- `feedUrl(name, params)`'s new optional argument produces the identical URL when called with none,
  which is the no-regression proof for the signature change.

## #63 `<head>` Atom/`vcs-git` discovery on repository pages

- **Server-side injection, not a client fill-in.** `/{repo}/*` shells are prerendered under the
  `__repo__` placeholder (#17), so there is no per-repo href at build time. Filling the `<link>`s the
  way `fillRepoShell` fills the heading was rejected: it would be invisible to exactly the non-JS
  consumers autodiscovery exists for, and `rel="vcs-git"` needs `clone_url_base`, api-side config the
  web build never sees. `shell.rs::serve_shell` injects them byte-wise before `</head>`, matching
  this project's "hand-build small fixed documents" stance (#12) rather than parsing HTML for three
  `<link>`s.
- **The repo segment is used raw, never decoded** — both the feed URL and the clone URL want it
  percent-encoded exactly as received, so this skips the decode/re-encode round trip. It still goes
  through `escape::xml_escape` (moved out of `feed.rs` into a shared module) before interpolation.
- **No repository-existence check** — the shell already answers 200 for a nonexistent repository, and
  adding a filesystem read to the static path to hide one inert `<link>` isn't worth the I/O.
- **Titles are fixed strings**, not the repository name, which is only available percent-encoded here.
- **`rel="vcs-git"` is omitted when `clone_url_base` is unset**, the same `null` rule `get_repo`'s
  `clone_url` follows.
- **`?ref=` is deliberately not reflected into the feed link** — that would mean parsing and
  re-encoding a query parameter inside the static-serving path.
- **`<ClientRouter />` needs no extra wiring**: `swapHeadElements` removes every non-persisted child
  of the outgoing `<head>` and appends the incoming document's, so navigation never leaves stale
  links behind. (Verified against Astro's source.)
- **`astro dev` never runs this**, so there is no dev-time equivalent and the e2e suite can't cover
  it — `shellFallback` only rewrites the request URL, never the response body, and `clone_url_base`
  is api-side config Astro has no access to. Coverage is Rust-side only; a comment in `Layout.astro`
  records why the feature appears nowhere in the web source. A permanent limitation, not a gap.

## #64 Repository index sort (`?sort=`, `AXGIT_REPOSITORY_SORT`)

- **`?sort=`, never cgit's `s=`** — axgit's own query vocabulary, as with `?ref=` (#61).
- **A single param carries both column and direction**: `name`/`desc`/`owner`/`idle`/`section`,
  optionally `-`-prefixed to flip. `idle`'s un-prefixed form is *descending* (most recently active
  first, as cgit does); every other key's is ascending. A leading `-` flips that key's own default
  rather than meaning "always descending", so `-idle` is oldest-first, not a no-op.
  `RepoOrder::parse`/`Display` round-trip exactly, which is what lets `ReposResponse.sort` echo the
  request's own spelling back.
- **`None` always sorts last, regardless of direction.** Reversing flips the comparison between two
  *present* values only, never the `Some`-vs-`None` branch — a repository with no `owner` shouldn't
  jump to the top under `-owner`. Ties break by `name` ascending, always, so the order is fully
  deterministic regardless of the snapshot's incoming order.
- **`idle` compares parsed timestamps, not the formatted string.** `format_rfc3339` preserves each
  record's own UTC offset rather than normalizing, so string order sorts wrong across offsets
  (`"+0900" > "-0500"` lexicographically even when the `-0500` instant is later).
- **Sorting moved out of `scan.rs` into the handler.** `scan_repos` sorting by name made every other
  order require a second full scan; it now returns directory-read order and `list_repos` sorts the
  clone of the `ScanCache` snapshot it already made to serialize.
- **Server default via `AXGIT_REPOSITORY_SORT`**, parsed with the same rule. `ReposResponse.sort`
  reports the configured default when the request omits `?sort=`, so a client never duplicates the
  server's default logic.
- **No response-cache work** — the list comes from `ScanCache` with a body-hash `ETag` (#6's
  carve-out), so a different `sort` yields a different `ETag` automatically.

## #65 Sortable repository-index column headers

- **Client-side re-sort, not a refetch** — `GET /repos` already returns everything in one response.
  `web/src/lib/repo-sort.ts` is a line-for-line mirror of `repo/sort.rs`'s rules (nulls-last
  regardless of direction, `idle`'s reversed default, name-ascending tiebreak, parsed instants), so
  a client-side sort and a server-side one of the same order never disagree.
- **`?sort=` is optional client state layered on the server's order**, not a value the client must
  carry. While unset, the rendered order — and which header shows `aria-sort` — is whatever the API
  returned (`ReposResponse.sort`, #64), not a client-recomputed guess. That is what makes an
  operator's `AXGIT_REPOSITORY_SORT` visible in the UI without the web build knowing it exists. An
  unparseable `sortParam` falls back as if absent.
- **Clicking a different column lands on that column's own default direction**, never inheriting the
  previous one; clicking the active column toggles. Written via `history.replaceState`, not
  `pushState` (#25's rationale).
- **`section` has no header button** — it is `groupBySection`'s heading, not a column, so sorting
  rows within an already-grouped table would do nothing visible. `?sort=section` still works.
- **No `DropdownMenu`** — importing base-ui's `Menu` pulls in a ~137 KB floating-ui chunk that #30
  deliberately keeps off every page but one. Plain `<button>`s inside each `<TableHead>` avoid it.
- **Accessibility fix found here**: `ui/table.tsx`'s `TableHead` rendered a bare `<th>` with no
  `scope`. Per HTML-AAM that should still map to `columnheader`, but Chromium doesn't do so reliably
  without an explicit `scope` (verified with a minimal repro outside this codebase). `scope="col"` is
  now the default. This was silently broken for every existing table, not just the sortable one.

## #66 `hide`/`ignore` repository flags

cgit's `repo.hide`/`repo.ignore` distinguish a repository absent from the index but still fetchable
by direct URL (`hide`) from one unreachable by any means (`ignore`). Fits the existing
`[cgit]`/`[axgit]` config-section invariant (#5), no new config surface.

- **Two flags, two different choke points, on purpose.** `hide` is enforced only in `scan.rs` (via
  `meta::should_list`), the sole producer of the list `ScanCache` holds — so a hidden repository never
  enters the snapshot, while direct access and clone keep working. `ignore` is additionally enforced
  in `open::open_named` (`meta::is_ignored`), the one function every per-repo handler and Smart HTTP
  call goes through, so an ignored repository 404s everywhere. `should_list` treats `ignore` as also
  implying "not listed", so a repository needn't set both.
- **`config_flag` sits alongside `config_value`** — same `[axgit]`-wins-over-`[cgit]` precedence,
  just `get_bool`, which already accepts every spelling cgit's booleans do. Unlike
  `handlers/mod.rs::parse_flag`, a bad value silently defaults to `false` rather than erroring: a
  config flag has no `400` channel, matching how an unparseable `section`/`owner`/`desc` resolves to
  `null`.
- **No API contract change** — neither flag is a response field; both are index/reachability
  behavior.

## #67 Per-repository `homepage`

- **`RepoInfo`/`RepoSummary` both gain `homepage: Option<String>`**, read through the same
  `config_value` lookup `section`/`owner`/`desc` use.
- **Only `http://`/`https://` values survive**; anything else — most importantly a `javascript:`
  URL — reads back as `null`. This is the first config-derived field that lands in an `href` rather
  than a text node, so an unchecked value in `cgit.homepage` would be stored XSS. Filtering at the
  read (`meta::is_http_url`) means the invariant holds for every consumer without each re-validating.
- **Rejected rather than erroring at scan time** — cgit doesn't validate `homepage` either, and
  failing a whole repository's listing over one bad config value is a worse failure mode.

## #68 Honour the configured `defbranch`

- **One choke point, not ten.** `resolve::resolve_commit` — already the single function every ref
  resolution goes through — substitutes the configured `defbranch` for the *literal* `"HEAD"` string
  before `revparse_single`, gated on an exact `refname == "HEAD"` check, so the config read only
  happens on that one input. That is what makes `/tree/HEAD/...`, `/blob`, `/raw`, `/blame`,
  `/archive/HEAD.*` and `/diff?from=HEAD` honour it for free. Only the four sites that called
  `repo.head()` directly, bypassing `resolve_commit` — `commits.rs`, `stats.rs`, `search.rs`,
  `feed.rs` — needed switching to `resolve::default_commit`, plus the summary's own `head` field,
  which would otherwise show a different tip than every other tab.
- **A stale or misconfigured `defbranch` degrades to HEAD, silently.**
  `meta::configured_default_branch` checks `find_branch(name, Local)` first, so a renamed or deleted
  branch doesn't 404 every ref-less request. Same stance as #67's scheme filter.
- **`default_branch` now reports the configured branch too** — no schema change, the field already
  existed as `Option<String>`.
- **Deliberately left on HEAD**: `meta::validator` (any ref movement should still signal a change)
  and `last_modified`'s `head_authordate` fallback (a push to a non-HEAD `defbranch` still touches
  the agefile).
- **No cache-key change.** `HEAD` and an explicit branch name stay distinct `params` strings, so
  resolving to the same commit just means two entries with identical bodies. A `defbranch` edit is
  config-only and invisible to the validator, so it inherits the `AXGIT_CACHE_RESPONSE_TTL`
  staleness ceiling — the documented safety net for exactly this class of out-of-band change.

## #69 Link each repository's homepage

- **Not a nav tab, unlike cgit.** `RepoNav.astro`'s tabs are prerendered under the `__repo__`
  placeholder (#17) and rewritten by `fillRepoShell`, which only builds `/{segment}/{sub}` — it has
  no notion of an arbitrary external URL, let alone one that may or may not exist per repository.
  `homepage` renders as a link in the two places that already show per-repository metadata: a
  `MetaItem` row on `RepoSummary` and a third `IconLink` in `RepoList`'s row actions. A deliberate,
  permanent difference from cgit, not a gap.
- **`target="_blank"` + `rel="noopener noreferrer"`, and only here.** `MetaLink` and `IconLink` gained
  an `external` prop defaulting to false; every other use of either points back into axgit itself.
  `homepage` is the first link in the app that genuinely leaves the site.

## #70 Site title, description, and readme

- **A new, non-repo-scoped `GET /api/v1/site`, not fields tacked onto `GET /repos`.** These describe
  the *deployment*, not any repository, and the readme needs its own response shape
  (`format`/`content`, mirroring `ReadmeInfo` without its repo-specific `path`). `title` is always
  present (falling back to `"Axgit"`); `description`/`readme` are `null` when unset.
- **The readme is read from the filesystem at request time, with no traversal check.**
  `AXGIT_ROOT_README` is operator configuration passed on the command line or environment, not user
  input reachable from a request parameter — the same trust boundary `AXGIT_REPO_ROOT` sits on.
  Capped at 512 KiB; over the cap, missing, or non-UTF-8 all degrade to `readme: null` rather than
  failing the response, the "one bad config value degrades one field" stance of #66/#67/#68.
- **Format is guessed from the configured path's extension**, reusing `ReadmeFormat` rather than
  duplicating the enum. Deliberately a *different* guess from `repo/readme.rs::CANDIDATES`'s
  name-based one — an operator-chosen path has no candidate list to match, only an extension.
- **Not cached at all** — no `ScanCache`, no moka entry: config is already in memory and the readme
  read is one stat plus one read. A body-hash `ETag` still gives clients a 304 path.
- **`<head>` injection generalized, not duplicated.** #63's helper became
  `inject_before_head_close(body, extra)`, a pure byte-splice over pre-built markup; `site_head_meta`
  builds the site metas and `serve_shell` concatenates both extras before a single splice.
- **Custom `axgit:` meta names, not overwriting the real `<title>`/`<meta name="description">`.**
  The shells are prerendered per route *shape*, so the real elements' content is baked in at build
  time — and the header **brand text** lives in `<body>`, which needs client-side filling regardless
  (`fillRepoShell`'s existing job). The custom metas smuggle config into the browser so one client
  script applies it to both places, instead of a server-side mechanism for `<head>` and a
  client-side one for `<body>` racing or disagreeing.
- **Injected into *every* shell — index, repo pages and 404** — unlike #63's repo-only `<link>`s: a
  site title is deployment-wide.
- **Bug found here: `GET /` never reached `serve_shell` at all.** `ServeDir`'s default
  `append_index_html_on_directories(true)` served `static_dir/index.html` directly for a bare `/` —
  the only route shape corresponding to a real on-disk directory — bypassing the `.fallback(shell)`
  closure entirely. Fixed with `.append_index_html_on_directories(false)`; no other route shape was
  affected, since none corresponds to a real directory.

## #71 Render the site title, description, and readme

- **A shared `ReadmeBody.tsx`, split out of `ReadmeView.tsx`.** Only the format-driven render
  (`markdown` → lazy `ReadmeMarkdown` in a `Suspense`, else `<pre>`) is shared with `SiteIntro`,
  which fetches its own readme and has no per-repo path to head with. Splitting keeps the ~135 KB
  lazy-loading boundary in exactly one place rather than duplicating the `Suspense`/`lazy` dance.
- **`ReadmeMarkdown`'s `repo` prop is optional.** Relative-link rewriting is meaningless for a
  site-level readme, which has no owning repository — `./guide.md` has no tree to resolve against.
  `repo === undefined` short-circuits `rewriteHref`/`rewriteSrc` to pass every URL through, rather
  than producing a nonsensical `/{repo}/...` href with an empty `repo`.
- **`SiteIntro` renders nothing when `title === "Axgit"` and `description`/`readme` are both `null`**
  — an unconfigured index page is byte-identical to before. This can't distinguish "nothing
  configured" from an explicit `AXGIT_ROOT_TITLE=Axgit`, but that is the same collapse
  `site.rs::effective_title` already makes server-side.
- **`fillSiteChrome`, not a second injection mechanism.** It lives beside `fillRepoShell` in
  `Layout.astro`'s init script, sharing its guard and `astro:after-swap` registration. It reads the
  `axgit:site-*` metas (#70) and applies them to three places: the header brand (persisted, so once
  per document), the real `<meta name="description">` (not persisted — it reverts to the shell's
  build-time default on every swap, so this genuinely needs reapplying), and — **only on
  non-repository pages**, checked via the absence of `[data-repo-name]` — `document.title`. A
  repository page's title stays `fillRepoShell`'s `name + suffix`: `RepoLayout.astro`'s
  `TITLE_SUFFIXES` are static build-time strings with no runtime site-title interpolation. A
  deliberate, revisitable limitation.
- **A one-time invocation, not just the listener.** The listener fires only after a client
  navigation; the first (hard) load needs a direct call. `fillRepoShell` gets that from
  `RepoLayout.astro`, which exists only on repository pages — `fillSiteChrome` needs it everywhere,
  so `Layout.astro` calls it from a small inline script right after `</header>`, by which point
  `[data-site-title]` is in the DOM. Astro's swap re-runs it too; harmless, the function is
  idempotent.
- **e2e coverage stops at the client half**, like #63's — `astro dev` never runs the server-side
  injection. The spec injects the metas by hand and calls the function directly, exercising the real
  client logic in a real browser without the Rust binary.

## #72 Submodule (`module-link`) links

- **Two sources, config first, `.gitmodules` as a fallback — an axgit extension cgit doesn't have.**
  cgit only reads `cgitrc`; axgit additionally reads `.gitmodules` from the resolved commit when no
  config template applies, using the mapped `url` verbatim if it's `http(s)`. A config template
  requires an operator to add one per path, while `.gitmodules` is content the repository already
  carries — a submodule pointing at a public forge gets a working link with zero configuration.
- **Four config keys, no new precedence machinery.** `axgit.<path>.module-link` →
  `cgit.<path>.module-link` → `axgit.module-link` → `cgit.module-link`, so path specificity is
  checked *before* section precedence — a `[cgit "<path>"]` entry beats a repo-wide `[axgit]` one.
  `git config` accepts a `/`- and `.`-bearing subsection (verified against the real binary): libgit2
  splits a key on its *first* and *last* dot, so a dotted path round-trips, and the subsection is
  case-sensitive, matching how a tree path should compare.
- **The first applicable source wins, even when its value turns out unusable — it never falls
  through.** If any config key is set at all, that is the answer (possibly `None` after validation);
  `.gitmodules` is consulted only when none is. This buys explicit suppression for free: an operator
  sets `axgit.<path>.module-link` to the empty string to kill a repo-wide template for one path
  without it degrading into "try the next source". A typo'd template also stays silently linkless
  rather than surprising the operator with a `.gitmodules` URL nobody asked for.
- **Template grammar: two `%s` placeholders (path, then sha), `%%` for a literal `%`; anything else
  involving `%` makes the whole template unusable.** A third `%s`, an unrecognized specifier, or a
  trailing lone `%` return `None` rather than substituting an empty string — "no link" is
  diagnosable in the tree view, "silently wrong link" is not. Unlike C's `printf` there is nothing
  meaningful to read for a third argument, so there is no cgit behavior to match.
- **Substituted values are inserted verbatim, not percent-encoded**, close enough to cgit's own
  substitution that an existing cgitrc value can usually be pasted in. Safe because the *scheme*
  comes from the template, not the substituted path, and the href guard runs on the **expanded
  result**: a gitlink literally named `javascript:alert(1)` under a bare `%s` template is still
  rejected.
- **Two distinct href guards, not one.** `meta::is_http_url` stays the strict `http(s)`-only check
  for `.gitmodules`, whose `url` is routinely a local path (`/srv/git/dep.git`) or an SSH remote —
  neither belongs in a same-site href. `submodule::is_link_href` covers the config-template result
  and additionally accepts a single leading `/`, because cgit's own documented example is exactly
  that shape (`/git/%s/commit/?id=%s`) — natural for an operator whose forge sits behind the same
  reverse proxy.
  - **A leading `/` immediately followed by another `/` or a `\` is rejected**, not just bare `//`:
    browser URL parsers fold both `//evil.com/x` and `/\evil.com/x` into protocol-relative (a
    backslash is normalized in the "special authority slashes" state), so either navigates off-site
    despite starting with a single `/`.
  - **A relative template is rejected outright.** cgit's other documented example
    (`./?repo=%s&page=commit&id=%s`) is this shape, and it is the one cgitrc value that genuinely
    cannot be pasted in unchanged: a relative href resolves against whatever tree path the browser
    is showing, so the same config value points somewhere different depending on nesting depth.
- **`.gitmodules` is hand-parsed, not read via git2's submodule API.** Verified first: git2 0.20.4's
  `Config` has no in-memory constructor (`open`/`add_file` are path-based), and libgit2's
  `gitmodules_snapshot` returns `GIT_ENOTFOUND` whenever `git_repository_workdir(repo) == NULL`,
  which is unconditionally true for every bare repository axgit opens. `Repository::submodules()` is
  a dead end here.
  The parser (`submodule::parse_gitmodules`) is pure and line-oriented: `[submodule "name"]` stanzas
  and `path`/`url` keys are matched case-insensitively, last-within-a-stanza wins (git's own
  semantics), and a `path` seen in an earlier stanza wins over a later duplicate. Keyed on `path`,
  deliberately **not** the stanza name — git allows the two to differ, and only `path` addresses
  anything in the tree. Capped at 64 KiB via an object-header stat (the `SYMLINK_TARGET_LIMIT`
  pattern), and an oversized file yields no links rather than a parse of truncated input.
  Deliberately unhandled, and documented in the module doc comment: line continuations, multi-line
  quoted values, `[include]`/`includeIf`, and relative `url` values (meaningful only against the
  superproject's own remote, which a bare repository doesn't have).
- **`entries_of` is left untouched; `list_tree` calls `submodule::fill_module_links` afterward.**
  Threading a resolution context through `entries_of` was rejected: the one optimization that matters
  (skip config and `.gitmodules` entirely when a listing has no gitlink) can only be expressed
  *after* the entries exist, and `GET /objects/{oid}` has no commit/path context to resolve a
  template against anyway. `fill_module_links` is infallible — every failure degrades this one field,
  the posture #67/#68 established — and loads `.gitmodules` at most once per listing, only when some
  gitlink's config lookup misses.
- **Frontend: `rel="noopener noreferrer"` and `data-astro-reload`, deliberately no
  `target="_blank"`**, and deliberately *not* #69's `external` prop. A `module-link` destination may
  be same-site (`/git/%s/…`) or off-site, and a per-row sniff would make two visually identical rows
  behave differently based on operator config. `rel` without `target` is inert for `noopener` but
  still suppresses `Referer`. **`data-astro-reload` is the load-bearing part**: `<ClientRouter />`'s
  click handler checks `el.dataset.astroReload !== undefined` before intercepting a same-origin
  click, so without it a same-site `/git/…` destination behind the same reverse proxy would get its
  HTML spliced into the axgit shell — same-origin is the only condition the router checks. The
  existing raw/archive links escape this only because their responses aren't `text/html`.
- **Caching splits along the same line the two sources do.** The config half is invisible to the
  HEAD/agefile validator, so a `module-link` edit with no push inherits the TTL ceiling (as for #67,
  #68). The `.gitmodules` half is repository content and invalidates normally.

## #73 Validate `--chart-2..5`

`--chart-2..5` had never been run through the `dataviz` skill's `validate_palette.js` because
nothing consumed them. Converted to hex, the shadcn-generated values turned out to be a *broken*
palette rather than merely an unverified one.

- **Measured, not assumed.** The four slots were Tailwind's teal ramp — one hue (182–188°) at four
  lightness steps, byte-identical in light and dark. Against `--background`, the validator hard-fails
  both modes: chroma floor (two slots below 0.10, reading as gray), the normal-vision floor (worst
  adjacent ΔE 7.9 against a floor of 15), and in dark mode the lightness band too. Slot 1 was
  re-checked alone and still passes, confirming #29's fix held.
- **Replacement: four hues from the skill's reference categorical palette, slot 1 unchanged.** All
  1680 orderings of 4 hues from the reference eight (excluding green, slot 1's family) were run
  through the validator for both modes against the real surfaces; 244 passed every hard gate. The
  order kept maximizes the worst adjacent CVD ΔE and needs no contrast relief in either mode: blue,
  orange, violet, red. Every hex is verbatim from the reference file. Light and dark are stepped
  separately now, unlike the old values.
- **The hexes were converted back to `oklch()` with a round-trip check**, since `global.css` is
  written in that form — `--chart-2` light needed a third hue decimal to avoid a one-bit drift.
- **No new consumer.** `StatsChart` stays single-series; `CommitGraph` (#33) and `RefBadges` (#34)
  keep their shape/icon encodings, which cite the method's identity-is-never-colour-alone rule
  directly rather than "the palette isn't ready" — this fix removes a stale justification, it doesn't
  change either decision.
- **Correction to #29's prose**: it describes light mode's original `--chart-1` as failing on
  "~1.4:1 contrast". The validator's contrast check is a non-blocking WARN; the actual hard FAIL in
  light mode was the lightness band, same as dark. The shipped fix was correct either way.

## #74 Single-binary build: `embed-web` Cargo feature

*(#88 flips this feature's polarity: embedding is now the default, and the opt-in `embed-web` name
became the opt-out `api-only`. The mechanics below — the `Assets` enum, `serve_embedded_file`,
`build.rs`'s rebuild trigger — are unchanged; only which build gets them for free changed.)*

Until now the only way to serve the frontend was `AXGIT_STATIC_DIR` pointing at a `web/dist`
directory — fine for the container, but a bare-metal install would ship two artifacts whose versions
must match exactly. `git` exec stays a runtime dependency regardless; this closes only the frontend
half.

- **`rust-embed`, behind an opt-in `embed-web` feature**, not `include_dir`: `EmbeddedFile::metadata()`
  gives a sha256 hash and a `mime_guess`-backed content type for free, both of which the serving path
  needs anyway. Opt-in so the default build has no compile-time dependency on `web/dist` existing.
  `debug-embed` is on unconditionally so `cargo test --features embed-web` exercises the same
  embedded-lookup path release builds use (without it `rust-embed` reads from disk in debug);
  `mime-guess` reuses the crate `repo/blob.rs` already depends on, so content types agree between
  both serving modes; `deterministic-timestamps` drops per-file mtimes so the binary doesn't vary
  with checkout time.
- **`api/src/assets.rs`'s `Assets` enum (`Dir(PathBuf)` / `Embedded`) is the single place that knows
  where the build is read from.** `Assets::resolve` prefers a configured `AXGIT_STATIC_DIR` and falls
  back to the embedded copy only when unset, so an operator can override a baked-in build without
  rebuilding. `shell.rs` takes `Assets` instead of a `PathBuf` and reads through `Assets::read`, so
  route-shape mapping, `<head>` injection and cgit-compat redirects are byte-for-byte shared between
  modes. `routes.rs` picks the outer fallback per mode; the embedded arm tries
  `assets::serve_embedded_file` then falls through to the shell, mirroring `ServeDir`'s own ordering
  without touching the filesystem.
- **`serve_embedded_file` mirrors `ServeDir`'s behavior deliberately**: a strong `ETag` (sha256,
  reusing `if_none_match`/`not_modified` for the 304 path) and percent-decoding. No explicit
  `..`-rejection is needed — `rust-embed`'s generated key set never contains a `..` segment, so
  `WebDist::get` returns `None` and the caller falls through to the shell exactly as an unmatched
  `ServeDir` path does.
- **`api/build.rs`, feature-gated on `CARGO_FEATURE_EMBED_WEB`.** `rust-embed`'s derive emits one
  `include_bytes!` per file, so rustc tracks content changes to files it already knows about but not
  files being added or removed — and every `pnpm --filter web build` produces new content-hashed
  `_astro/*` filenames. `cargo:rerun-if-changed=../web/dist` closes that gap. Feature-gated because
  pointing `rerun-if-changed` at a path that may not exist forces an unconditional rebuild.
- **libgit2 does *not* honour `GIT_CONFIG_GLOBAL` here.** Read from the vendored libgit2 1.9.6
  source: `config_path_global()` consults it only when the repository is opened with
  `GIT_REPOSITORY_OPEN_FROM_ENV`, and `open_named` uses `Repository::open_bare`, which doesn't set
  that flag. `$HOME/.gitconfig` and `/etc/gitconfig` *are* read regardless, and
  `validate_ownership_config()` looks up `safe.directory` through exactly that stack. Consequence for
  a bare-metal install: run the service as the user that **owns** the repositories, so the ownership
  check passes outright and no `safe.directory` entry is needed at all.
- **Size cost, measured**: a release build with `--features embed-web` is ~24.0 MB vs ~20.0 MB
  without (`web/dist` is 4.0 MB across 132 files) — roughly 1:1, expected since most of `web/dist`'s
  weight is already-compressed JS and `woff2`.

## #75 Single-binary deploy packaging: release tarball, systemd unit, Jenkins release stage

- **`scripts/make-release.sh`** builds the frontend, then
  `cargo build --release --locked --features embed-web --target "$TARGET"`, and stages the binary
  with `packaging/axgit.service`, `axgit.env.example`, `INSTALL.md` and `LICENSE` into
  `release/axgit-$VERSION-$TARGET.tar.gz` + `release/SHA256SUMS` *(superseded by #89 — the tarball
  now stages the binary and `LICENSE` only)*. `VERSION` comes from
  `api/Cargo.toml` rather than being duplicated; when `TAG_NAME` is set the script asserts it matches
  `v$VERSION` and fails **before building** — a tag/manifest skew should stop the release, not ship a
  mislabelled tarball.
- **Target: `x86_64-unknown-linux-musl`, built natively on the CI agent — not a Docker-stage build.**
  Matches the container's static-linking posture (#22) without adding a Docker dependency to a
  pipeline that assumes a bare Node/Rust toolchain. musl needs its own C compiler for the vendored C
  dependencies, so the script requires `musl-gcc` on `PATH`, sets `CC_x86_64_unknown_linux_musl`
  explicitly rather than letting `cc` fall back to the host's glibc compiler, and checks the rustup
  target first — failing with exact remediation commands rather than a raw linker error. `TARGET` is
  overridable purely so the staging/tar/checksum logic can be exercised on a dev machine.
- **`packaging/axgit.service`** *(now inline in README.md#systemd-install, #89)*: `Type=exec`,
  `EnvironmentFile=-/etc/axgit/axgit.env` (the `-` makes
  it optional, matching the all-env-var design). `User=`/`Group=` default to `git`, with a comment
  pointing at #74's ownership finding. Hardened with the standard systemd sandboxing directives while
  leaving process spawning and network access open — axgit forks `git` for archive/upload-pack, so
  those can't be sandboxed away. **`ProtectHome=read-only`, deliberately not `yes`**: libgit2 still
  reads `$HOME/.gitconfig` as part of the ownership-check config stack (#74), and a fully hidden home
  would make that lookup silently see nothing.
- **`packaging/axgit.env.example` mirrors README.md's configuration table field-for-field** so the
  two don't drift — both describe the same `Config` struct *(superseded by #89 — the mirror was the
  drift; README.md's table is now the only list)*.
- **`packaging/INSTALL.md` ships inside the tarball**, since it is needed at install time on a host
  that may never have cloned this repo *(superseded by #89 — it is a README.md section instead)*.
- **Jenkins releases on `v*` tag builds only, via `archiveArtifacts`** — producing and retaining the
  artifact, matching the pipeline's existing "test-only, artifacts archived" posture. The `Release`
  stage runs after `Test`, so a broken build never produces a tagged artifact even if the tag was
  pushed *(superseded by #91 — the pipeline is gone; `make-release.sh` and its `TAG_NAME` guard
  are what a replacement has to drive)*.

## #76 CI coverage and a binary smoke check for the single-binary release path

Nothing between "code compiles" and "a `v*` tag ships a tarball" had ever exercised the `embed-web`
feature or run the binary it produces — the `Release` stage's build would have been the first to
touch that code, and `make-release.sh` never ran what it built.

- **A sequential `Embedded build` stage, after `Test` and before `Release`, running on every build**
  rather than gated to tags — a break should surface on the next ordinary commit, not the next
  release attempt. It runs `pnpm --filter web build` (also the only place CI runs the *production*
  Astro build at all — Playwright's `webServer` uses `pnpm dev`), then
  `cargo clippy --features embed-web --all-targets -- -D warnings`, then `cargo test` scoped to
  `--lib --test embedded_assets_test --test static_shell_test`: clippy `--all-targets` already proves
  every embed-gated file compiles, and re-running the rest of the integration suite would spend the
  timeout on work the parallel `Test` stage did. This stage builds for the host target; only
  `Release` builds musl.
- **A smoke check in `make-release.sh`**, right after the existing executable-exists check: when the
  host can actually run a `$TARGET` binary (same OS and architecture — a musl-vs-glibc host libc
  difference doesn't matter for a statically linked binary), it runs `"$BINARY" --version` and
  asserts the output is exactly `axgit $VERSION`. A mismatch fails before anything is staged. When
  the host can't run the target binary — the common case on a Linux agent building for musl — the
  check is skipped with a one-line explanation rather than failing, since making it mandatory would
  require a matching runner or emulation layer that doesn't exist.
- Deliberately did **not** add a second agent or emulation layer: this closes the "silently never
  compiled" gap, not the "never run on the actual release target" one.

## #77 Immutable `Cache-Control` for content-hashed `_astro/*` assets

- **One mode-independent layer, not two duplicated header-setting sites.** Astro's default
  `build.assets` puts every content-hashed file — and nothing else — under `/_astro/`, so
  `assets.rs::HASHED_ASSET_PREFIX` plus a `starts_with` check is an exact test, not a heuristic.
  `assets::immutable_cache_for_hashed_assets` is an `axum::middleware::from_fn` handler wired at the
  outermost `Router` layer, so it sees the final response regardless of which arm produced it.
- **Applied only on a 200 or 304**, checked against `response.status()` after `next.run` rather than
  trusted from the request path: a `/_astro/*` request for a file no longer in the current build
  falls through to the 404 shell, and that 404 must never be marked immutable — a client hitting a
  stale link would otherwise cache the miss for a year.
- **Reuses `handlers::IMMUTABLE_CACHE_CONTROL`** rather than a second constant — the same header
  full-sha-addressed API responses get, and visibly the same promise for the same reason.

## #78 Decided against stripping the release binary

`scripts/make-release.sh` builds with the same `release` profile as the container (no
`strip = true`), so a panic backtrace on either deploy shape still names real functions. Stripping
would shrink the binary (~24.0 MB → an expected 15–18 MB) at the cost of that symbol information;
for a self-hosted, single-tenant service where the operator debugging a panic *is* the person
reading the backtrace, debugging value outweighs the size saving. Not revisiting unless tarball size
itself becomes a problem.

## #79 aarch64-unknown-linux-musl release leg

`scripts/make-release.sh` builds both `x86_64-unknown-linux-musl` (native) and
`aarch64-unknown-linux-musl` (cross) by default, producing two tarballs and one `SHA256SUMS`.
Cross-compiling a musl target with `cc`-crate C dependencies has several non-obvious failure modes,
checked against `cc` 1.4.0's and `pkg-config` 0.3.33's source:

- **The actual C dependency set is `libgit2-sys`, `libz-sys`, `liblzma-sys` and `zstd-sys`.**
  `bzip2-sys` is gone — `bzip2 0.6` switched to the pure-Rust `libbz2-rs-sys`; `libz-sys` comes in
  transitively via libgit2-sys, not `flate2` (which uses `zlib-rs`). No `cmake`/`bindgen` in the
  lock, so neither is a CI-agent prerequisite.
- **cc-rs's env lookup order is `CC_<triple-dashes>` > `CC_<triple_underscores>` > `TARGET_CC` >
  `CC`.** Exporting `CC_<triple_underscores>` therefore shadows any `TARGET_CC` an operator sets, so
  the override mechanism is honouring a pre-set `CC_<triple_underscores>`, not adding a second
  variable. The dashed form is moot — it isn't a valid bash identifier.
- **cc-rs's built-in cross prefix table only knows one aarch64 name**: `prefix_for_target` hardcodes
  `aarch64-linux-musl` with no existence check, while the x86_64 musl case actually probes PATH. So
  musl.cc/Homebrew `musl-cross` naming works with zero env vars, and
  `messense/macos-cross-toolchains` naming (`aarch64-unknown-linux-musl-gcc`) needs the explicit
  export the script does. The script probes both conventions, cross-only names first.
- **`CARGO_TARGET_<TRIPLE>_LINKER` is required for the cross leg, not optional.**
  `rustc --print target-spec-json` shows no `linker` key and `linker-flavor: "gnu-cc"`, so rustc
  drives the final link through PATH's `cc`, which cannot link foreign-arch objects. Left unset on
  the native leg. `AR_<triple>` is exported best-effort (essential from a macOS host whose cctools
  `ar` can't index ELF); `RANLIB` deliberately isn't — cc 1.4.0 never calls it internally.
- **The single biggest hazard: never set `PKG_CONFIG_ALLOW_CROSS`, `PKG_CONFIG`, or
  `PKG_CONFIG_SYSROOT_DIR`.** `git2` doesn't enable libgit2-sys's `vendored` feature, so its build.rs
  always tries a system libgit2 via pkg-config first. The *only* reason the build ends up vendored at
  all is that `pkg-config`'s `target_supported()` refuses to run when `host != target` unless one of
  those three overrides it. Setting any of them flips libgit2-sys and libz-sys onto the host's glibc
  `.pc` files, producing a binary that looks statically linked but silently isn't.
- **Two-pass structure**: every target's toolchain is resolved and validated before any build starts,
  so a missing cross compiler fails immediately rather than after the first leg. `release/` is wiped
  up front and `SHA256SUMS` written once from the explicit list of tarballs produced — not a glob —
  so a stale file can never be checksummed alongside the current release.
- **The smoke check has a runner ladder**: `CARGO_TARGET_<TRIPLE>_RUNNER` (cargo's own convention) →
  native execution → `qemu-<arch>-static`/`qemu-<arch>` on PATH (no `-L <sysroot>` needed, the binary
  is statically linked musl) → a registered `binfmt_misc` handler → skip. The skip path is
  deliberately a hard "give up and say so" rather than "try anyway and catch the failure": a
  genuinely broken binary also fails to execute, so a blanket fallback would turn the exact failure
  #76 built this check to catch into a silent pass. **Honest caveat**: on a stock x86_64 agent with
  no `qemu-user-static`, the aarch64 binary is still built and shipped, just unexecuted — installing
  it is a recommended, not enforced, agent prerequisite.

## #80 `Dockerfile` → `Containerfile` + symlink, base images qualified with their registry

- **`Containerfile` is the real file; `Dockerfile` is a committed relative symlink** (mode `120000`).
  podman/buildah look for `Containerfile` first, and `docker build .` follows the symlink
  transparently, so both are first-class without maintaining two copies.
- **Base-image `ARG`s spell out their registry** (`docker.io/library/node:...`). Docker already
  resolves unqualified names this way, so nothing changes for `docker build` — but podman/buildah
  consult `registries.conf`'s `unqualified-search-registries` and otherwise fall back to an
  interactive "which registry did you mean" prompt, which fails outright in any non-TTY build.

**Deliberately not ported from the older cgit-based `git-web` image** — recorded so a future session
doesn't rediscover these as gaps:

- `git-daemon` (apk): git-web needed it for nginx's FastCGI `git-http-backend`. axgit execs
  `git upload-pack`/`git archive` directly, both in the base `git` package; `git-http-backend` is
  never invoked.
- `VOLUME ["/srv/git"]`: for axgit that failure mode is the wrong one to hide — an anonymous,
  writable volume silently standing in for a forgotten `:ro` bind mount contradicts the read-only
  posture. A missing mount should surface as "no repositories found", not a quiet writable fallback.
- `/home/git/.gitconfig` + `HOME=/home/git`: axgit's single `/etc/gitconfig` (#22) already covers
  both the exec call sites and libgit2, since both read the system-wide config.
- Narrowing `safe.directory` from `*` to `/srv/git/*`: rejected again, same reasoning as #22 —
  `AXGIT_REPO_ROOT` is runtime-configurable, so a narrower pattern would silently break any
  deployment that points it elsewhere.
- TLS termination and the HTTP→HTTPS redirect: delegated to an external reverse proxy either way
  (#10).
- **Out of scope, not a container concern**: git-web's nginx also enforced edge policy with no axgit
  equivalent — a 405 outside GET/HEAD/POST, and a 444 for AI-crawler/empty-`User-Agent` patterns.
  Reproducing it belongs in the git-compose stack's reverse proxy, not this repository.

## #81 Site logo and favicon

Reverses part of the "no analogue planned" cgit-parity note, which had conflated arbitrary head/body
injection (still out of scope) with a single operator-supplied image (a narrow, well-bounded feature
cgit also treats as config).

- **`AXGIT_LOGO`/`AXGIT_FAVICON` accept an `http(s)://` URL *or* a filesystem path**, dispatched by
  scheme prefix (`branding.rs::BrandingAsset::parse`) — unlike cgit's URL-only `logo`/`favicon`.
  cgit runs as CGI behind a general-purpose web server, so pointing at an on-disk file just works;
  axgit ships as a single container or binary with no sibling static-file server, so a URL-only
  option would mean "self-host your logo somewhere else". The path form reads the file and serves it
  through axgit at `GET /api/v1/site/logo`/`/favicon`, mirroring `AXGIT_ROOT_README` (#70): operator
  config, read at request time, no traversal check (the value never comes from a request).
- **A closed extension allowlist decides both the servable `Content-Type` and whether the asset
  loads at all** (`svg`/`png`/`ico`/`jpg`/`jpeg`/`gif`/`webp`/`avif`). This is a security boundary:
  the bytes are served same-origin with `nosniff` but no further inspection, so an operator
  accidentally pointing `AXGIT_LOGO` at an `.html` file must never become stored XSS. Deliberately
  not `mime_guess` (already a dependency) — that table covers far more than "safe to serve as a site
  image", and this list must stay small and auditable rather than inherit whatever `mime_guess`
  recognizes next. An unrecognized extension degrades to `404`.
- **1 MiB cap** (`BRANDING_ASSET_LIMIT`), the same reasoning as #70's 512 KiB readme cap, sized for
  an image.
- **`AXGIT_LOGO_LINK` is validated separately, and a bad value degrades to unset rather than failing
  the logo.** Only `http(s)://` or a root-relative `/…` path is accepted, reusing
  `meta::is_http_url` (as #67 and #72 do) plus `module-link`'s root-relative allowance. A
  protocol-relative `//…` is explicitly rejected despite starting with `/` — it resolves to any host,
  an open redirect, not a same-origin link.
- **The favicon gets a real server-side `<link rel="icon">` injected into `shell.rs`'s `<head>`
  splice; the logo travels as `axgit:logo`/`axgit:logo-link` metas read client-side by
  `fillSiteChrome`.** Different mechanisms for a reason: a browser fetches favicon `<link>`s while
  still parsing `<head>`, before any script runs, so a client-side swap would always fetch axgit's
  default first and flash it. The logo has no such race — it is `<body>` chrome filled once per
  document, exactly the split #70/#71 drew.
- **A configured favicon strips the shell's own default `<link rel="icon">` pair
  (`shell.rs::strip_default_icon_links`) rather than only adding a third.** Leaving both would leave
  the browser to pick per its own tie-breaking rules. Byte-oriented (scans for `<link` tags
  containing `rel="icon"` and drops each whole tag) — this file is never third-party HTML, so a full
  parser buys nothing (#12's stance).
- **`favicon_type` is resolved once in `routes.rs` from `BrandingAsset::content_type()` on the
  *parsed* asset, not guessed from the served `href`.** The file form's href is the fixed,
  extensionless `/api/v1/site/favicon` route, so the extension has to be read off the configured path
  (or, for the URL form, the URL with query/fragment stripped) before `href()` throws it away. An
  earlier version guessed from `href` and silently omitted `type=` for every locally-served favicon.
- `web/public/robots.txt` `Allow`s both new routes ahead of the blanket `Disallow: /api/v1/` — a
  site's own logo and favicon should stay crawlable.

## #82 Render the site logo and configured favicon

- **Logo and title are siblings inside the header brand `<a>`, not a replacement.**
  `[data-site-title]` moved onto an inner `<span>` with an `<img data-site-logo alt="" hidden>`
  beside it. `alt=""` because the adjacent title text already names the site. `fillSiteChrome`
  unhides and sets `src` only when `axgit:logo` is present, so an unconfigured header is unchanged —
  and the accessible name still comes from the span, so existing lookups resolve.
- **No favicon logic on the web side at all** — `shell.rs` already rewrote the `<link rel="icon">`
  server-side, and a redundant client-side path would reintroduce the flash #81 avoided.
- **`fillSiteChrome` gains the logo reads, not a second function** — one more field on an
  already-idempotent, already-registered function.
- **A configured-but-unloadable logo gets an `onerror` handler that re-hides the `<img>`**, rather
  than leaving a broken-image icon in the header. The meta is injected whenever `AXGIT_LOGO` is
  *configured*, independent of whether the file can actually be served; `shell.rs` has no cheap way
  to confirm that without re-reading the file on every shell response, which would undermine the
  "swap the file, no restart needed" point of its `no-cache`.
  **The favicon has no equivalent recovery** — a misconfigured `AXGIT_FAVICON` still strips the
  default `<link rel="icon">` pair (the strip only checks "is a favicon configured"), so the
  deployment shows no icon until the value is fixed. Left as-is: there is no `onerror`-driven
  "restore what was removed" without the shell retaining the markup it just stripped, this matches
  cgit's behavior for a broken `favicon`, and the failure is self-inflicted operator config, logged
  with a `tracing::warn!` either way.

## #83 Compose repository titles from the configured site title

Closes the limitation #71 recorded on purpose: `TITLE_SUFFIXES` baked `"— Axgit"` in at Astro build
time, so a deployment with `AXGIT_ROOT_TITLE` set still got `git-compose log — Axgit` in the tab.

- **`TITLE_SUFFIXES` drops the trailing site name** (`" — Axgit"` → `" — "`), and `fillRepoShell`
  appends the live site title — the `axgit:site-title` meta when present, else the same `"Axgit"`
  literal — closing over the exact value `fillSiteChrome` uses for non-repository pages. Repository
  titles stay entirely `fillRepoShell`'s job (#71's split is unchanged), just no longer blind to
  `AXGIT_ROOT_TITLE`.
- The static per-page `title` props remain correct build-time placeholders: `fillRepoShell` runs from
  an `is:inline` script before first paint and overwrites them unconditionally.

## #84 Replace the scaffold favicon with an axgit mark

Two defects dating to the original web scaffolding commit: `web/public/favicon.svg` was the stock
Astro starter logo, and `web/public/favicon.ico` was a 32×32 PNG with an `.ico` extension rather
than an actual ICO container.

`favicon.svg` is now PtCookie's own git mark — a fixed dark-circle badge with a green branch/node
glyph, with no `prefers-color-scheme` swap, since the dark circle is legible against both
browser-chrome themes. `favicon.ico` is a real 16×16 + 32×32 ICO container generated by a one-off
script (`web/scripts/make-favicon.mjs`, rasterizing the SVG through the Playwright already installed
as a `web/` devDependency and hand-writing the ICONDIR/ICONDIRENTRY header) rather than adding a
dependency for a one-time asset — the same "keep the generator alongside its output" precedent
`make-fixtures.sh` sets. It lives under `web/` rather than the repo-root `scripts/`, which has no
`package.json`, so a bare `node` there can't resolve the `playwright` import.

## #85 Default header logo to axgit's own mark

- **The default is always axgit's own `/favicon.svg`, not a configured `AXGIT_FAVICON`.**
  `AXGIT_LOGO` and `AXGIT_FAVICON` stay the independent options #81 designed — an operator who wants
  a different image in the tab than in the header loses nothing. Only the *unconfigured* default
  changes.
- **Baked into the markup, not set from `fillSiteChrome`.** `[data-site-logo]`'s `src` is
  `/favicon.svg` from first paint; `fillSiteChrome` only overwrites it when `axgit:logo` is present.
  This avoids a pop-in on the header's un-persisted first load.
- **A configured logo that fails to load falls back to the default mark instead of hiding the image**
  (amends #82's `onerror`: `logo.hidden = true` → `logo.src = DEFAULT_SITE_LOGO`). Why a load can
  fail is unchanged from #82; only the degraded state changed, since a blank header slot is worse
  than the mark an unconfigured deployment already shows. `onerror` clears itself before setting the
  fallback so a failure loading the default can't loop.

## #86 Blank repo metadata reads as unset; unsectioned group moves to the top, sorted A–Z

Repositories with no category showed up in the *middle* of the index. Two gaps that only surfaced
combined: some repos' git config had `section = ` (key present, value blank), which `config_value`
returned as `Some("")`; and `groupBySection` only special-cased `section === null` as the "Other"
bucket, so `Some("")` formed its own group, sorted among the named ones, with an empty `<h2>`.

- **Blank config values normalize to `None` in the API, not just the frontend.** A
  `.filter(|value| !value.trim().is_empty())` in `read_repo_info`'s `meta` closure covers
  `section`/`owner`/`desc` — all free-text operator fields rendered as-is, unlike `homepage`
  (filtered by `is_http_url`) or `defbranch` (validated against real branches). **`config_value`
  itself is untouched**: `module_link_template` calls it too, and there an empty per-path override is
  a deliberate "suppress the repo-wide template" signal (#72), not "unset". Fixing this at the API
  layer means `GET /repos`, `GET /repos/{repo}` and the Atom feed all agree on what "no section
  configured" means, instead of the frontend reinterpreting a wire value the contract never promised
  was meaningful.
- **The unsectioned group sorts first, and named groups sort A–Z.** The original design put
  unsectioned last, on the assumption most repositories would eventually get a category; in practice
  a large plurality never will, and burying them at the bottom of a long page is worse than
  surfacing them ahead of deliberately curated categories. The group keeps an `sr-only` `<h2>` so the
  section landmark stays consistent, but carries no visible label — position alone communicates it.
  - Group comparison uses plain code-point ordering (`a.section < b.section`), matching
    `repo-sort.ts` and the API's `str::cmp`-based `sort.rs` — not `localeCompare`, which would let
    the two orderings drift apart on non-ASCII section names.
  - `?sort=section` no longer influences which group appears first; group order is fixed by the rule
    above. It still governs order within a group, via the `name` tiebreak.
- Fixtures set `cgit.section` to an explicit blank value, since otherwise nothing reproduced the
  actual bug shape. The *mocked API response* fixture deliberately wasn't changed the same way — by
  the time a response reaches the frontend the API has normalized blank to `null`, so a `""` there
  would test a wire shape the real API no longer produces.

## #87 TOML config file (`--config`/`AXGIT_CONFIG`, `/etc/axgit/axgit.toml`)

axgit exists to replace cgit, so it should be configurable the way cgit is — one readable,
commentable file holding the whole site configuration, alongside the existing flags and `AXGIT_*`
variables.

- **TOML, not a cgitrc-style `key=value` parser.** Reading real `cgitrc` syntax would let an existing
  file be pointed at directly, but that parity is shallow: the *keys* are what an operator
  recognizes, and those carry over regardless of syntax. A hand-rolled parser would add a bespoke
  escaping/typing story for `listen` addresses and byte counts. `toml` is one small parse-only
  dependency (axgit never writes TOML back) with types and spans in its errors.
- **Sections are organizational; keys keep cgit's names.** `[site]` groups branding/index chrome,
  `[cache]` the cache knobs, everything else is top-level. `[site]`'s keys stay
  `root-title`/`root-desc`/`root-readme`/`logo`/`logo-link`/`favicon` rather than the shorter names
  grouping would invite — a value copied out of a real `cgitrc` has to be recognizable at a glance.
  `[cache]`'s keys are axgit's own, since cgit's `cache-*` options don't map onto them.
- **Precedence is CLI flag > environment variable > config file > default.** The other order would
  mean a config file mounted into the container silently reconfiguring a deployment always driven by
  its `ENV`. Layering the file *under* the environment keeps every existing deployment unchanged and
  makes the file purely additive.
- **The merge reads clap's `ValueSource`**, rather than restructuring `Config` into layers of
  `Option`. `Config::load` applies a file value only where `value_source` reports `DefaultValue` or
  nothing — the whole precedence rule in one predicate, with the clap struct, its defaults and its
  `--help` text untouched. `merge` takes that predicate as an `impl Fn(&str) -> bool` over arg ids so
  the rule is unit-testable without standing up a process environment. (Arg ids are the derive's
  field idents — `cache_scan_ttl_secs`, not `cache-response-ttl`; renaming one without the other is
  the one way to break this silently, so the ids appear as literals next to the fields they set.)
- **Discovery: `--config`/`AXGIT_CONFIG` if given, else `/etc/axgit/axgit.toml` if it exists.** A
  named-but-missing file aborts startup (the operator asked for it); a missing default is silent.
- **Unknown keys warn and are ignored**, rather than failing startup as `deny_unknown_fields` would:
  a file derived from a real `cgitrc` — carrying `scan-path`, `enable-*`, `snapshots`, `css` and the
  rest of the ~80 options axgit has no equivalent for — still boots, with one warning naming each
  dropped key. The known-key list is hand-maintained and exhaustive (`config/file.rs`'s
  `TOP_LEVEL_KEYS`/`SITE_KEYS`/`CACHE_KEYS`), the same auditable-on-its-own stance as
  `branding.rs::content_type_for_extension`. Cost: the file is parsed twice, once as a `toml::Table`
  to find stray keys and once into the typed struct.
- **A malformed file *is* fatal**, unlike the branding/readme paths that degrade to `None`. Blast
  radius is the difference: a broken logo path spoils one response, while an unparseable config file
  means every setting in it is silently absent and the whole deployment runs on values the operator
  didn't choose. Same for an invalid `repository-sort`, validated by the same `parse_repository_sort`
  the flag's `value_parser` uses.
- **`main.rs` installs the tracing subscriber before loading config**, or the unknown-key warnings
  would be emitted with no subscriber and vanish.
- **Not adopted: cgit's `repo.*` blocks and `include=`.** Repositories are discovered by scanning
  `repo-root`, and their metadata comes from each bare repo's own `config` — a core invariant, not
  something the site config file should override. TOML has no include of its own, so adding one would
  mean inventing a directive rather than adopting a format's.
- **The image's `ENV` is `AXGIT_STATIC_DIR` alone** *(superseded by #88 — the image no longer sets
  it at all; kept here for why it was pinned in the first place)*. `AXGIT_REPO_ROOT` and
  `AXGIT_LISTEN` were byte-identical to the binary's clap defaults, so pinning them bought nothing
  and only made them unsettable from a mounted config file — an `ENV` line is *always* "set", and
  env beats file. General rule for this image: **don't restate a default the binary already has.**
  - `AXGIT_STATIC_DIR` was the exception: it had no clap default (unset meant "serve no frontend" in
    the then-default, non-`embed-web` build), and its correct value was a fact about the image, not
    an operator preference. It was the one key a mounted config file couldn't set.

## #88 Shell-route manifest emitted by the build; `embed-web` flipped to `api-only`

Two separate maintenance costs, closed together because the second only became affordable once the
first removed the reason a non-embedded build existed at all.

- **The route-shape table (#17) is no longer hand-duplicated in Rust.** `web/src/lib/shell-routes.ts`
  is now the single authored table; `web/src/lib/shell.ts::shellFor` (the `astro dev` middleware)
  reads it directly, and an Astro integration (`astro.config.mjs`'s `shellRoutes()`) emits it to
  `dist/shell-routes.json` on `astro:build:done`. `api/src/shell.rs::ShellRoutes::load` deserializes
  that file instead of matching a hand-written `match` — `shell_for` walks the table in order instead.
  Adding a route now means touching `src/pages/[repo]/` and `shell-routes.ts`; the same integration
  hook cross-checks the two (a declared route with no built shell, or a built shell nobody declared,
  fails the build), closing the gap a silent third drift point would otherwise leave.
- **`ShellRoutes::load` falls back to `ShellRoutes::fallback()`** — a Rust-literal copy of the same
  table — when the manifest is missing or fails to parse, rather than failing startup. In practice
  this only fires for an `AXGIT_STATIC_DIR` pointed at a build predating this file, or at a directory
  that was never an axgit frontend build at all; every embedded build and every freshly built
  `AXGIT_STATIC_DIR` carries the real manifest, so the fallback is a safety net, not a maintenance
  burden — it doesn't need to track a route added only to `shell-routes.ts`.
- **`embed-web` flipped to `api-only`: embedding is now the default, not opt-in.** The old shape had
  it backwards for what deployments actually do — every container and release build already turned
  the feature on, so "opt-in" bought nothing but a Cargo flag everyone had to remember to pass, while
  the *actual* default (no frontend at all, `cargo build` with no flags) was never shipped anywhere.
  `rust-embed` is now an unconditional dependency; `cfg(feature = "embed-web")` became
  `cfg(not(feature = "api-only"))` throughout `assets.rs`/`routes.rs` (mechanically inverted, same
  branches). `api/build.rs` requires `web/dist` to exist unless `api-only` is set, with an error
  message naming both fixes (`pnpm --filter web build`, or add the feature).
- **The Containerfile no longer sets `AXGIT_STATIC_DIR`.** The `api` build stage now copies the
  `web` stage's `web/dist` in at `../web/dist` (relative to its own `/app/api` workdir) before
  `cargo build`, and the runtime stage no longer copies `web/dist` in separately or pins the env var
  — the frontend is already in the binary. This is the deployment-shape change #87 flagged as the
  cost of making `AXGIT_STATIC_DIR` file-settable: it's no longer set at all, so there's nothing left
  for a mounted config file to conflict with.
- **`api-only` stays a real, tested build**, not a vestigial escape hatch: CI's `api` branch
  (Jenkinsfile) runs `cargo clippy --features api-only --all-targets` and
  `cargo test --features api-only --lib --test static_shell_test` alongside the default build, so a
  change that breaks the pure-API path is caught the same run it's introduced *(the intent stands,
  but no CI runs these today — see #91)*.
- **`scripts/make-release.sh` and the Jenkinsfile's Release stage drop `--features embed-web`** —
  the tarball build already ran `pnpm --filter web build` first, so the default build now does what
  the explicit flag used to. The Jenkinsfile's old standalone "Embedded build" stage is gone; its
  job (proving the frontend-bundled path compiles and its tests pass) is now just what the default
  `api` branch already does.

## #89 `packaging/` folded into README.md; one config example at the repository root

Four files existed to describe settings and install steps that README.md already described. Each
was a second place to update, and #75's own "mirrors README.md field-for-field" bullet named the
problem it was creating: `axgit.env.example` and `axgit.toml.example` both enumerated the same
`Config` struct as README.md's configuration table, so a new setting meant editing three lists to
say one thing.

- **`packaging/INSTALL.md` → README.md's `systemd install` section**, and **`packaging/axgit.service`
  → an `ini` code block inside it**, comments and all. The unit's comments *are* its documentation
  (the `User=`/`Group=` ownership note, `ProtectHome=read-only` vs `yes`), so they moved with it
  rather than being summarized away; the one bullet that step 2 already explains in prose was cut.
- **`packaging/axgit.env.example` deleted outright.** README.md's configuration table lists every
  `AXGIT_*` variable with its config-file key and default — the template added nothing but a place
  for the two to disagree.
- **`packaging/axgit.toml.example` → `axgit.toml` at the repository root**, which doubles as the
  local-dev config (`--config ./axgit.toml`, since axgit only auto-reads `/etc/axgit/axgit.toml`).
  One commented file now serves both "what can I set?" and "what do I run locally?", so the example
  is exercised by ordinary development instead of only being read at install time.
- **The release tarball is the binary plus `LICENSE`.** The install steps are no longer *in* the
  tarball, which is the real cost here — #75 shipped `INSTALL.md` precisely because the install host
  may never have cloned the repo. Accepted deliberately: the tarball is downloaded from a page that
  can link README.md#systemd-install, and one authoritative copy of the steps beats a second copy
  that silently ages. Revisit by generating `INSTALL.md` from the README section at release time if
  that assumption ever stops holding.
- **`compose.example.yaml` → `compose.yaml`.** The header comment already said it is illustrative
  and not wired into any deploy process, and the `.example` infix defeated editor/tooling
  recognition of the compose schema for no benefit the comment wasn't already providing.

## #90 API and architecture docs folded into the component READMEs

Same motive as #89, one level up: `docs/API.md` and `docs/ARCHITECTURE.md` described code that
lives in a specific directory, while `api/` had no README at all — someone opening it found neither
the build commands nor the contract the handlers implement. And `ARCHITECTURE.md`'s "Build/deploy"
section restated README.md's Deployment / Single-binary build / Configuration sections, so the
`AXGIT_*` list existed in two places again.

- **`docs/API.md` → `api/README.md`'s `API (v1)` section** (a `git mv`, so the history follows).
  Its headings dropped one level to nest under the new `##`; the heading *text* is unchanged, so
  every `#anchor` still resolves. Prose that said "this document" now says "this section", and the
  two references the file made to itself and to `ARCHITECTURE.md` became in-document pointers.
- **`ARCHITECTURE.md`'s Backend section → `api/README.md`'s `Design` section**, keeping
  `### Caching` verbatim: `api/README.md#caching` is cited from `cache.rs`, `repo/meta.rs`,
  `handlers/mod.rs`, `Cargo.toml`, and `cache_test.rs`.
- **Its Frontend section → `web/README.md`'s `Design` section**, with the route list refreshed —
  it still marked blame as planned and predated the `diff`/`search`/`stats`/`tag`/`object` pages.
- **Its Background and Overall layout → README.md's new `Architecture` section**; its Build/deploy
  section was folded into the existing Deployment section, keeping only what README.md didn't
  already say (image stages and layer caching, runtime packages, the non-root user and
  `[safe] directory = *`, JSON logs to stdout). The `AXGIT_*` enumeration was dropped outright —
  README.md's configuration table already carries every variable with its config-file key and
  default.
- **`docs/` is now just `DECISIONS.md` and `ROADMAP.md`**, the two records that belong to the
  repository rather than to one component and whose `#NN` anchors are cited from both `api/` and
  `web/` sources.
- Roughly 90 `docs/API.md` citations across `api/src`, `api/tests`, `web/src` and `DECISIONS.md`
  were repointed at `api/README.md`. `openapi.json` and `web/src/lib/api/types.ts` carry the same
  strings but are **generated**, so they were regenerated rather than edited — the only reason this
  rename touches them at all.

## #91 Jenkins pipeline removed ahead of a GitHub Actions replacement

The `Jenkinsfile` described a pipeline tied to one self-hosted agent and its hand-provisioned
prerequisites (both musl rustup targets, `musl-tools`, an aarch64 cross musl gcc, `qemu-user-static`,
a Playwright browser install). CI is moving to GitHub Actions; the Jenkins definition is deleted
first, in its own commit, so the replacement is written against the current tree rather than ported
line by line from a file it is meant to supersede.

- **Deleted, not left in place alongside a second definition.** Two pipeline files claiming to be
  the source of truth for what CI runs is the failure mode worth avoiding — one of them silently
  rots. **Consequence, stated plainly: between this commit and the GitHub Actions workflows, nothing
  runs the checks automatically.** Until then they are the local commands in AGENTS.md, backed by
  lefthook's pre-commit hooks.
- **What the replacement owes, carried by the entries that specified it** rather than restated here:
  the `web`/`api` split with `--features api-only` alongside the default build (#88), the production
  Astro build being CI's only production `pnpm --filter web build` (#76), the release smoke check
  and its "skipped when the host can't execute `$TARGET`" caveat (#76, #79), and `v*`-tag-only
  release artifacts (#75).
- **Nothing outside the `Jenkinsfile` was rewired.** `scripts/make-release.sh` keeps its `TAG_NAME`
  guard — the variable is a plain CI contract, not a Jenkins one, and a GitHub Actions job sets it
  from `github.ref_name` just as readily. Comments in `make-release.sh` and `web/vitest.config.ts`
  that reached for Jenkins to explain *why* (a shared agent running the `web` and `api` jobs
  concurrently, which is what `retry: process.env.CI ? 2 : 0` exists for) now say "CI", since the
  reasoning survives the provider change.
- `api/src/cgit_compat.rs`'s reference to Jenkins is unrelated and stays: it names Jenkins' `cgit`
  Repository browser as the external producer of cgit-shaped URLs that #35 redirects.

## #92 GitHub Actions: CI, `v*` release tarballs, and a GHCR image

The replacement for the pipeline #91 deleted, split across `.github/workflows/ci.yml`,
`release.yml` and `image.yml` plus a `dependabot.yml`. The checks themselves are the same commands
the Jenkinsfile ran — what changed is where they run, and what happens after they pass: a `v*` tag
now publishes both the release tarballs and a container image, which is the CD half the old
pipeline never had.

- **The mirror is what shapes the CI file.** `git.ptcookie.net` is the origin; this GitHub
  repository is a mirror that git-server's post-receive hook **force**-pushes to. So **no workflow
  may write to the repository** — a commit pushed by Actions is erased by the next mirror push,
  which rules out the usual auto-format / commit-the-regenerated-file patterns for `openapi.json`
  and `types.ts`. CI only checks; lefthook and the author remain the gate. For the same reason an
  approved PR (Dependabot's, typically) is **pulled locally and pushed to the origin** rather than
  merged through the GitHub UI: the commits arrive here identical, so GitHub marks the PR merged on
  its own, whereas a UI merge would create a merge commit the origin has never seen. `push` is
  scoped to `main` because PR branches live in this same repository and an unscoped trigger would
  run every PR twice; `concurrency` cancels in-flight runs because a force-pushed rewrite makes
  them meaningless.
- **`web-build` is its own job, ahead of `api`.** The default cargo build bakes `web/dist` into the
  binary (#74, #88) and `api/build.rs` fails outright without it, so the production Astro build
  can't live inside the `api` job's own steps if the two are to be separable — it runs once and
  hands `web/dist` over as an artifact. `web` runs in parallel and needs neither that artifact nor
  a running axgit: the e2e suite mocks `/api` via `page.route` against `astro dev`. `--with-deps`
  on the Playwright install is new — GitHub's runner image ships none of the webkit/firefox system
  libraries the old self-hosted agent had.
- **Release builds go native per architecture** (`ubuntu-latest` + `ubuntu-24.04-arm`, the latter
  free on public repositories) instead of cross-compiling both targets on one x86_64 machine. This
  deletes the whole cross-toolchain prerequisite #79 documented — an aarch64 cross musl gcc, the
  `CC_`/`AR_`/`CARGO_TARGET_*_LINKER` exports, `qemu-user-static` — down to one `musl-tools`
  install per runner, since `musl-gcc` is `make-release.sh`'s first native candidate. It also
  turns #76's smoke check from "skipped whenever the host can't execute `$TARGET`" into something
  that **always** runs: the binary that ships has been executed on its own architecture.
- **`SHA256SUMS` is written by the `publish` job, not by the build.** `make-release.sh` still emits
  one, but a matrix job only ever sees its own tarball; the script's contract that the checksum file
  covers every tarball produced now lives in the job that collects them all. The per-target files
  are discarded, and the script is otherwise unmodified.
- **The image publishes on `v*` tags only**, as a manifest list assembled from two natively built,
  push-by-digest images. Deployment stays manual (the git-compose stack pulls and restarts by
  hand), so a stream of per-commit images would have nothing consuming it; tagging follows
  metadata-action's defaults — `X.Y.Z`, `X.Y`, and `latest` withheld from prereleases via
  `latest=auto`. Two consequences worth writing down: **a GHCR package is private on first publish
  even for a public repository** and has to be flipped once by hand, and `metadata-action`
  overwrites the `Containerfile`'s `org.opencontainers.image.source` — pointing at the origin for a
  local build and at the GitHub mirror here, which is what links the package to a browsable
  repository. Only the registry layer cache (`type=gha`) is used: BuildKit does **not** persist
  `RUN --mount=type=cache` mounts through it, and those mounts still earn their keep for the
  from-source build on the deployment host, so the `Containerfile` is left alone.
- **`pnpm --filter web check` gained `astro sync && tsc --noEmit`.** Type checking existed only in
  lefthook's pre-commit hook, which meant it was skipped by anything that didn't commit. Putting it
  in `check` rather than in a separate CI step keeps the local command and CI running the same
  thing. `astro sync` is required, not decorative: without `.astro/types.d.ts` — a generated file,
  present locally but absent in a clean checkout — `tsc` fails on `import.meta.env`.
- **Dependabot covers `cargo`, `npm` and `github-actions`**, grouped and weekly. Known risk
  recorded in the file itself: Dependabot's pnpm parser cannot read the multi-document
  `pnpm-lock.yaml` pnpm 11 emits once an env-lockfile document is present
  (dependabot-core#14919). This repository's lockfile is still single-document, so updates work
  today; if that changes, the `npm` entry gets disabled and the other two stay.
- **No provenance attestation yet.** `actions/attest` would need the manifest-list digest threaded
  out of the merge job plus `id-token`/`attestations` permissions, and nothing consuming this image
  verifies attestations — revisit if that changes rather than carrying the complexity now.
