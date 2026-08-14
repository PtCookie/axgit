# Decision history (ADR-lite)

Recorded cumulatively, in numeric order. To reverse a decision, don't delete it — leave a
"Superseded by #N" note.

## #1 Replace Cgit with a custom frontend

Reimplement two roles — a read-only web UI and Smart HTTP clone — in a new stack. Push continues
to be handled by the existing git-server (SSH), so the web app has no auth or write path.

## #2 Backend: Rust + git2 (libgit2) + axum

- Go/Rust performance is essentially a non-issue at this scale — the bottleneck is how git objects
  are accessed and cached.
- Rust was chosen for preference/learning purposes. git2 is stable; gitoxide's API is still in
  flux, so it was ruled out.
- axum as the web framework: the standard in the tokio ecosystem, and tower middleware makes
  caching/compression easy.
- Heavy operations (archive, upload-pack) use git binary exec (Gitea-style hybrid).

## #3 Frontend: Astro (static) + React + shadcn/ui + Tailwind

No SSR adapter. Dynamic data is fetched client-side from React islands. If SSR is ever needed,
that's a separate decision to reverse this one (it changes the number of deployed containers).

## #4 Monorepo + single container

- web and api are tightly coupled via the API contract → manage them in one repo with atomic
  commits.
- A single Rust binary serves static files + API + Smart HTTP → one container. Removes
  nginx/fcgiwrap. Added to git-compose as a single submodule.

## #5 Preserve compatibility with existing data sources

- Metadata: continue reading the repo config's `[cgit]` section (an `[axgit]` section takes
  precedence if present). This avoids having to modify git-server's `git-init`/`post-receive`.
- Last activity: agefile `info/web/last-modified` → falls back to HEAD authordate.

## #6 Caching: cgit's TTL approach, with an improved validator

Response cache key `(repo, endpoint, params)`, validator `HEAD sha + agefile mtime`. Sha-pinned
responses are treated as immutable. Clients use ETag/immutable. (ARCHITECTURE.md#caching)

Finalized during implementation (adopting moka):

- The validator is git2 open, then `head().target()` compared against the agefile's
  **raw `SystemTime`** (`repo/meta.rs::Validator`). Parsing `.git/HEAD`/`packed-refs` directly was
  not adopted — it's fragile around symbolic refs, and the open cost is low anyway.
- The caching layer lives in a **shared handler helper** (`handlers/mod.rs::cached_response`), not
  tower middleware. Whether a response is immutable is only known after ref resolution, params
  normalization/content-type differ per endpoint, and error responses must never be cached.
- The ETag is a strong ETag generated from the validator (its exact format isn't part of the
  contract). Request coalescing (`get_with`) isn't used — it doesn't fit the validator
  invalidation flow, and redundant computation at this scale is acceptable.
- **TTL (default 300s) is a safety net**: a staleness ceiling for out-of-band changes the
  validator can't see (manual config edits, a non-HEAD push without hooks). Per-entry body cap
  1 MiB, total capacity tracked in bytes (default 32 MiB).
- Exceptions: the repo list keeps `ScanCache` (TTL) + a body-hash ETag; raw is excluded from the
  cache (large binary data); archive can't be cached since it's streamed + gets a weak ETag.

## #7 Toolchain

- Package manager: **pnpm** (workspace). The root `pnpm-workspace.yaml` bundles `web` as a
  package, and JS commands run from the root as `pnpm --filter web <script>`. api is Rust, so it's
  outside the workspace.
- git hooks: **lefthook** — a single binary managing both Rust and JS hooks from one config
  (replaces husky+lint-staged).
- Testing: web uses **vitest** (browser mode, `@vitest/browser-playwright` provider +
  `vitest-browser-react`) + Playwright e2e; api uses cargo test + fixture repo integration tests.
- Commits: Conventional Commits (English).

## #8 API style

- REST JSON, base `/api/v1`. The normative document for the contract is `docs/API.md`; the
  machine-readable OpenAPI spec is generated from code (#15).
- Commit author emails aren't exposed — only a hash for the avatar seed is provided.
- Pagination uses a commit-sha cursor.

## #9 v1 scope

Included: repo list/summary/refs/log/tree/blob/raw/README/commit/diff/archive/Atom feed/Smart HTTP
clone. Included but implemented last: blame. Excluded (for later): stats (commit statistics
graphs), repository search, HTTP push.

## #10 TLS is delegated to a separate reverse proxy container

The Axgit container serves HTTP only. A reverse proxy service (e.g. nginx) is added separately to
the git-compose stack, with the certbot volume mounted there. TLS is not added to the Rust server.

## #11 Cgit filter replacements (frontend rendering)

Astro's built-in Shiki/markdown are build-time only, so runtime data is handled by client
libraries instead:

- Syntax highlighting: Shiki client-side (lazy-loaded grammars, skipped for large files).
- Markdown README: react-markdown + remark-gfm + rehype-sanitize (sanitizing is mandatory — repo
  content is untrusted input).
- reStructuredText/man: not rendered, shown as plain text (`<pre>`). No JS renderer exists, and
  reintroducing a docutils-scale dependency was rejected.
- Commit message linkification: regex-based linkify in React.
- Avatars: locally generated avatars seeded from an email hash instead of Gravatar (DiceBear
  recommended, no external requests).

## #12 archive/feed implementation

- archive uses **`git archive` exec** (the #2 hybrid policy). The ref is resolved server-side and
  only the **full sha** is passed on the command line — user input never reaches the exec
  arguments, so there's no injection surface. stdout is streamed chunked via `tokio-util`'s
  `ReaderStream`; a separate task collects stderr and waits on the process to prevent zombies.
- The Atom feed's XML is **generated by hand** (with escaping helpers). A runtime XML crate isn't
  worth pulling in for one fixed-structure flat document. quick-xml is a dev-dependency only (used
  in tests to verify well-formedness).
- No base URL configuration is added. The feed's absolute URLs are reconstructed from the
  `X-Forwarded-Proto`/`X-Forwarded-Host`/`Host` headers (a consequence of the #10 reverse-proxy
  policy). The entry `<id>` needs to stay stable regardless of host so feed readers don't show
  duplicates, hence the `urn:sha1:{sha}` format.

## #13 Smart HTTP: spawn `git upload-pack --stateless-rpc` directly

- Spawns **upload-pack directly** rather than going through `git http-backend` (CGI) — reusing the
  #2 hybrid policy and archive's tokio::process + `ReaderStream` + reaper pattern. No CGI env setup
  is needed, and since the process itself is pinned to upload-pack, receive-pack is
  **structurally** unreachable (the read-only invariant is enforced by structure, not code review).
- Only the git dir path resolved/validated by `open_named` is passed on the command line (user
  input never reaches exec arguments — same principle as #12). protocol v2 is supported by
  sanitizing the `Git-Protocol` request header and passing it through as the `GIT_PROTOCOL` env
  var.
- A gzip'd request body is **fully buffered and decompressed synchronously with flate2** (inside
  spawn_blocking). Negotiation data tops out at a few MB even on huge repos, so streaming gunzip
  (async-compression) would add complexity for no benefit. Limits: 8 MiB compressed
  (DefaultBodyLimit), 64 MiB decompressed (guards against zip bombs).
- Writing to stdin runs in a separate task to eliminate write/read deadlock risk; stdout is
  streamed.
- The moka response cache + ETag rollout wasn't bundled with this work and was split into a
  separate task (Smart HTTP responses are no-cache, orthogonal to caching).

## #14 blame: git2 `blame_file` (not exec)

- Finalizes an item ROADMAP had deferred as "decide after benchmarking vs. `git blame` exec": went
  with **git2 `blame_file`**. Rationale: (1) ARCHITECTURE.md already lists blame under git2's
  responsibilities, (2) unlike the exec approach, the path is never exposed as a command-line
  argument at all (archive/Smart HTTP follow an "exec only gets a resolved sha" principle to
  prevent injection, but blame isn't exec in the first place, so that constraint doesn't even
  apply), (3) the cost of writing a new `git blame --line-porcelain` output parser outweighs the
  fact that git2's API already provides hunk-level structs directly. No separate benchmark was
  run — if libgit2 proves slow on large histories (the same concern noted for log's path filter),
  an exec fallback will be reconsidered then.
- Binary/over-1-MiB detection reuses the same limit as the blob endpoint
  (`blob::BLOB_CONTENT_LIMIT`, shared via the `classify` helper) — there's no reason the "this
  file can't be viewed" judgment should differ between blob and blame from the user's perspective.
- Each range includes the commit `summary` (since the commit is already fetched, this costs almost
  nothing extra, and it saves the frontend from calling the commit detail API once per range while
  drawing the blame gutter).
- Rename/copy tracking: see #36 — the premise stated here originally (that libgit2 doesn't follow
  renames) turned out to be wrong; whole-file renames were tracked internally all along, just not
  surfaced in the response.

## #15 OpenAPI spec generated from code via utoipa

- Introduces OpenAPI **right before** the web implementation, something #8 had deferred as "revisit
  later." This is meant to avoid ever creating the debt ROADMAP's web section had anticipated —
  "manually define `types.ts` and keep it in sync with API.md."
- **Hand-written `openapi.yaml` is rejected.** It would become a second source of truth separate
  from the code and drift (in fact even the prose document, API.md, had already drifted in two
  places — a stale `501 not_implemented` entry, and a `charset=utf-8` that didn't match reality).
  Instead, the spec is generated from code via `#[utoipa::path]` + `#[derive(ToSchema)]`, and the
  result is committed to `docs/openapi.json`. `api/tests/openapi_test.rs` verifies both snapshot
  equality and "routed operations == spec operations," so an endpoint missing its annotation fails
  CI. Regeneration: `AXGIT_UPDATE_OPENAPI=1 cargo test --test openapi_test` (no separate binary
  needed).
- **`utoipa-axum`'s `OpenApiRouter` auto-collection is not used.** tree/blob/raw/blame/archive are
  all a single `{*rest}` catch-all in axum, with the ref/path boundary resolved via longest-match
  at request time — auto-collection would document `/tree/{rest}`, which doesn't match API.md's
  `/tree/{ref}/{path}` contract. Paths are written by hand in `api/src/openapi.rs` — this means
  dual maintenance with the router, but the operation-list test above covers that cost.
- **Swagger UI is served via `utoipa-swagger-ui`'s `vendored` feature** (`/swagger-ui`). Assets
  ship inside the crate, so neither build nor runtime makes external requests — the same principle
  behind #11's decision to generate avatars locally, and it means the self-hosted environment
  works offline too. The cost is a few extra MB in the binary. Since the API is read-only and
  unauthenticated, there's no exposure risk, so no separate on/off flag was added.
- **Closed string sets were converted to real enums**: `DiffStatus`, `EntryKind`, `LineOrigin`,
  `ReadmeFormat`. `&'static str`/`char` would just show up as `string` in the spec, with no
  documentation or generated-type benefit. JSON output stays byte-for-byte identical via serde
  rename (`LineOrigin` still serializes as `" "`/`"+"`/`"-"`). For the same reason,
  `#[schema(required = true)]` was added to `Option` fields that are always serialized, so the
  generated type is `field: T | null` rather than `field?: T | null`.
- Error bodies moved from `serde_json::json!` literals to `ErrorResponse`/`ErrorBody` structs —
  something was needed to attach a schema to, and this also guarantees the spec can never drift
  from actual responses.
- web generates `web/src/lib/api/types.ts` from `docs/openapi.json` via `openapi-typescript`
  (`pnpm gen:types`). Since it's a generated file, it isn't edited directly and is excluded from
  eslint.

## #16 Static build + SPA fallback

Superseded by #17.

- In Astro static mode, `{repo}` is the URL's first segment, so per-repository routes can't be
  enumerated at build time (`getStaticPaths` would need the repo list at build time, and that
  differs per deployment). Subpaths like `/{repo}/tree/...` are handled by client-side routing;
  the api serves `index.html` as a fallback for those requests to boot the SPA shell.
- **Note**: in axum, routes that are `nest("/api/v1")`'d or merged inherit the outer
  `fallback_service`. Wiring this up requires an explicit JSON 404 fallback on the api router,
  otherwise `/api/v1/bogus` would get the SPA's HTML shell instead of a proper JSON error.
- This commit (the repo list page) records the policy only. Actually wiring up the api and
  introducing the client-side router happens together with the per-repository pages commit.

## #17 Prerendered page shells per route shape (refines #16)

- **Astro pages are the routing layer again.** `src/pages/[repo]/index.astro` and
  `[repo]/refs.astro` are prerendered once under a reserved `getStaticPaths` param (`__repo__`,
  `web/src/lib/shell.ts`), producing `dist/__repo__/index.html` and `dist/__repo__/refs/index.html`
  alongside `dist/index.html` and `dist/404.html`. `output: "static"` and the single-container
  shape (#3, #4) are unchanged — this is a build-output and routing change only, not a
  deployment-shape change.
- **The server maps path *shape* → shell** (`api/src/shell.rs`), replacing #16's blanket
  `ServeFile(index.html)`. `ServeDir` still serves real files first (`_astro/*`, favicons); what is
  left is matched as `[]` → `index.html`, `[repo]` → the repo shell, `[repo, "refs"]` → the refs
  shell, anything else → `404.html` with a real `404`. Unmatched paths therefore stop answering
  200. The `{repo}` segment is matched but never used to build a filesystem path, so it carries no
  traversal surface. #16's note about `nest`ed routers inheriting the outer fallback still stands —
  the `/api/v1` JSON 404 is still required.
- **The client-side router is deleted** (`lib/router.ts`, `App.tsx`). Per-page chrome —
  repository heading, tab bar, active tab, `<title>` — is static HTML from
  `RepoLayout.astro`/`RepoNav.astro`. Only the repository *name* is unknowable at build time, so
  one `is:inline` script in the repo layout fills the heading, the tab `href`s and `document.title`
  from `location.pathname`'s first segment. `is:inline` (not a bundled `<script>`) because bundled
  scripts are deferred and would flash an empty heading.
- **Data islands stay `client:only="react"`, now with `slot="fallback"` skeletons.** `client:load`
  was considered and rejected: the shell is built with a placeholder param, so a hydrated island
  would receive `"__repo__"` as a prop and would have to re-derive the repository from `location` in
  an effect — three render passes, broken component tests, and a hydration-mismatch footgun right
  before log/tree/blob land. `client:only` + a fallback slot puts the same skeleton in the
  prerendered HTML with no matching constraint.
- **Cost: the mapping table exists three times** — `api/src/shell.rs::shell_for`,
  `web/src/lib/shell.ts::shellFor` (also driving the `astro dev` middleware in `astro.config.mjs`),
  and the `src/pages/` tree. Each carries a "change both together" comment and the two functions
  share an identical test case table. Single-sourcing this would require SSR, which #3 rules out.
  Adding a route means touching all three.
- **A repository literally named `__repo__` is shadowed** — its requests resolve to the shell files
  directly rather than through the mapping. Benign today (they are the same files it would have
  been mapped to) and documented rather than defended in code; it only becomes a real collision if
  a future route shape lacks a corresponding shell file.

## #18 Web `?ref=`-only ref selection, anchor-based log pagination, DiceBear identicons

Resolves the item ROADMAP had deferred as "confirm again when starting" for the log/commit pages,
plus two smaller choices made alongside them.

- **Ref selection on the web is `?ref=` only** — path segments after `/tree`, `/blob`, `/log`,
  etc. are always the file/commit path, never a candidate ref. The alternative (reimplementing the
  API's branch/tag longest-match, `repo/resolve.rs::resolve_ref_path`, in the client so URLs could
  look like `/{repo}/blob/main/src/main.rs`) was rejected: it would require fetching the refs list
  before rendering any file/log page just to resolve the URL, doubling a request that today only
  the API needs, and ARCHITECTURE.md already commits to "ref selection is unified via the `?ref=`
  URL query." `shellFor`/`shell_for` (#17) therefore only ever need to know a request's *shape*,
  never resolve a ref boundary — keeping them pure segment-count matches.
- **Log pagination is a plain anchor (`Older →`), not client state.** Consistent with #17: there is
  no client-side router, so "loading the next page" is a full page load to
  `/{repo}/log?cursor=...`. Only a forward link is rendered — the commit cursor is one-directional
  (docs/API.md), and a "Newer" link would need the frontend to remember cursor history itself; the
  browser back button already covers that case, matching cgit's own log pager UX.
- **Avatars: `@dicebear/collection`'s `identicon` style**, seeded from `email_hash` and rendered as
  a `toDataUri()` `<img>` — finalizes what #11 had only named ("DiceBear recommended"). `@dicebear/core`
  is pinned to `^9.4.3` rather than the newest major (`^10`): `@dicebear/collection` only declares a
  peer range of `^9.0.0`, and several of its style packages (transitively bundled, not just
  `identicon`) import an `escape` helper that v10's `@dicebear/core` no longer exports — installing
  the two majors together breaks Vite's dependency pre-bundling. Revisit the pin once
  `@dicebear/collection` publishes a v10-compatible release.

## #19 `/{repo}/tree` + `/{repo}/blob` pages, Shiki highlighting, dev shell-fallback fix

- **`shellFor`/`shell_for` (#17) generalized from a fixed-arity tuple match to a segment-slice
  match** (`match segments.as_slice()` in Rust, an equivalent length/prefix check in TS) — tree and
  blob paths have unbounded depth (`/{repo}/tree/a/b/c`), which the old fixed 4-lookahead-segment
  match couldn't express. `/{repo}/tree` (root, no path) is valid; `/{repo}/blob` (no path) is not
  — there's nothing to display — and 404s like the other unmatched shapes.
- **Fixed a latent dev-only bug found while wiring the tree/blob shells**: `astro.config.mjs`'s
  `shellFallback` middleware used `path.extname(pathname) === ""` to decide whether a request was
  an app route, skipping the rewrite for any path with a dot in it. A blob path
  (`/{repo}/blob/src/main.rs`) has a dot but is still an app route, so this 404'd in `astro dev`/
  Playwright while production (`api/src/shell.rs`, which has no such restriction) worked fine. The
  heuristic was replaced with an actual `public/`-file existence check (normalized and prefix
  checked against `public/` to rule out `..` escaping), mirroring what `ServeDir`'s real-file-first
  fallback order does in production.
- **Shiki, `shiki/core` + `shiki/engine/javascript` (the JavaScript RegExp engine, `forgiving:
  true`)** — not the default Oniguruma/WASM engine. Avoids shipping a ~500 KiB `.wasm` asset; the
  tradeoff is reduced grammar accuracy for a handful of complex languages, accepted without
  benchmarking a specific one. Both `github-light`/`github-dark` themes are loaded and tokenized
  together (`codeToTokens` with `themes: {light, dark}`), and each token's `htmlStyle` (a `color` +
  a `--shiki-dark` custom property) is used directly as a React inline `style` object — matching
  Shiki's documented dual-theme CSS-variables pattern. `global.css` adds
  `.dark .shiki-code span { color: var(--shiki-dark) !important; }` to complete it; there's no
  dark-mode toggle wired up yet, so this only activates once one exists (`.dark` is shadcn's
  existing, currently-unused, convention).
- **Language grammars are lazy-loaded per file** (`lib/format/highlight.ts`'s `LANG_LOADERS`,
  keyed by extension) — only about twenty common languages are mapped; anything else renders as
  plain text. Highlighting is skipped above **512 KiB or 5000 lines** regardless of language.
  Tokens are rendered as React nodes (`<span style=...>`), never `dangerouslySetInnerHTML` —
  repository content is untrusted input, the same rule `lib/format/linkify.tsx` follows.
- **No new API contract** — tree/blob/raw already existed (an earlier session). The web only added
  `getTree`/`getBlob`/`rawUrl` to `lib/api/repos.ts`, reusing `apiFetch`. `ref` continues to be
  `?ref=`-only (#18); when absent, the literal string `HEAD` is passed as the API path's `{ref}`
  segment rather than fetching the refs list to resolve a default.

## #20 `/{repo}/blame` page

The final v1-scope web page — `/{repo}/tree`/`/{repo}/blob` (#19) covered browsing, this closes the
last gap versus cgit.

- **The blame response carries no file content** (docs/API.md) — `BlameView` fetches
  `getBlame`/`getBlob` in parallel (`Promise.all`, same shape as `CommitView`'s detail + diff) and
  renders `blob.content` with the blame ranges laid over it.
- **`CodeBlock.tsx` gained an optional `gutter` prop** rather than a parallel blame-specific code
  viewer — a `(GutterCell | null)[]` indexed like the content's lines, where `GutterCell.rowSpan`
  merges one `<td>` down over a whole blame range (the table already has one `<tr>` per line, so
  this only needed one more column). `gutter` omitted (every existing caller) renders identically
  to before the prop existed. `BlameView`'s `buildGutter` fills the array from `BlameRange[]` and
  pads any trailing display-only lines (e.g. the empty element `content.split("\n")` produces after
  a final newline) with a blank one-row cell so the column stays aligned.
- **Gutter content is compact, cgit-style**: short (7-char) sha linking to `/{repo}/commit/{sha}`,
  relative time, author name — the commit summary is a `title` tooltip only, no avatar, to keep the
  code column wide.
- **Entry is blob-only, no dedicated nav tab** — `BlobView` gained a "Blame" link next to
  Raw/History; `RepoNav.astro`'s four tabs are unchanged, and `RepoLayout.astro`'s active-tab
  mapping treats `blame` like `blob` (highlights "Tree"). Routing otherwise follows blob's pattern
  exactly: `web/src/pages/[repo]/blame/[...path].astro`, one more `shellFor`/`shell_for` segment-
  slice arm requiring at least one path segment (`api/src/shell.rs`, `web/src/lib/shell.ts`).
- **`treeHref` moved out of `PathBreadcrumbs.tsx`** into a new `web/src/lib/repo-href.ts`, alongside
  new `blobHref`/`blameHref` siblings — three call sites (`TreeView`, `BlobView`, `BlameView`) all
  need repo-page hrefs now, so centralizing avoided a fourth ad hoc `` `/${encodeSegment(repo)}/...` ``
  construction.
- No API contract change — `docs/API.md`/`docs/openapi.json` untouched; `schemas.ts` gained
  `BlameInfo`/`BlameRange` aliases, `lib/api/repos.ts` gained `getBlame`.

## #21 README rendering + archive/feed links

Closed the last gap in ROADMAP.md's "Next up": `readme`/`archive`/`feed.atom` were implemented
endpoints with no web UI pointing at them at all.

- **react-markdown + remark-gfm + rehype-sanitize, no `rehype-raw`** — matches #11's plan exactly.
  Omitting `rehype-raw` means raw HTML embedded in a README is dropped rather than sanitized-and-
  kept; accepted, since re-introducing it would mean trusting `hast-util-sanitize`'s schema to catch
  everything, and repository content is untrusted input (same rule Shiki tokens and
  `lib/format/linkify.tsx` already follow — never `dangerouslySetInnerHTML`).
- **Relative links/images are rewritten to repository URLs** (`lib/markdown-url.ts`'s
  `isExternalUrl`/`resolveRepoPath`, pure and unit-tested) — without this, every relative
  `./docs/x.md` or `images/logo.png` in a README 404s, since the browser resolves them against the
  page URL (`/{repo}`), not the repository tree. `<a href>` maps through `treeHref` (trailing `/`)
  or `blobHref`, `<img src>` through `rawUrl` — all three already existed. A link/image resolving
  outside the repository root (`../../escape`) or already absolute/external passes through
  untouched.
- **`ReadmeView` is its own island**, not folded into `RepoSummary` — a 404 (`path_not_found`, no
  candidate found, or `ref_not_found`, unborn HEAD) is `docs/API.md`'s normal "nothing to show"
  outcome for this endpoint, not an error, and keeping it a separate fetch means that 404 can't
  touch `RepoSummary`'s own error state.
- **Markdown code fences get Shiki highlighting**, reusing `lib/format/highlight.ts`: the
  tokenizing tail was split out into a private `tokenize(code, lang)`, with `highlightCode`
  (path-derived language) and the new `highlightFence` (fence info-string-derived, via new
  `languageForFence`, sharing `languageForPath`'s extension-alias table) both thin wrappers over it.
  `ReadmeView`'s `pre` component override intercepts fenced blocks (inline code isn't wrapped in
  `<pre>`) and renders them through a local `MarkdownFence`, the same swap-in-once-highlighted
  pattern as `CodeBlock.tsx`, minus line numbers/gutter.
  - **Pitfall hit while wiring this up**: react-markdown's `Components` substitution means a hast
    `<code>` node's React element `type` becomes the *provided component function* once a `code`
    override is set, not the string `"code"` — so `pre`'s check for "is my child a fenced code
    block" must compare `child.type` against the `code` component's own reference (hoisted to
    module scope, `InlineCode`), not `=== "code"`. Also, `children` passed to `pre` is always an
    array (hast-util-to-jsx-runtime's convention) even for a single child, so it must be unwrapped
    before the `isValidElement` check. Missing either one silently falls back to unhighlighted
    plain text with no error — only caught via manual browser verification, not the component
    tests (which is why `ReadmeView.test.tsx` now asserts on the actual rendered
    `pre.shiki-code > span[style]`, not just the fence's text content).
- **archive/feed are HEAD-only links on the summary page** (`RepoSummary.tsx`, two new `dl` rows:
  tar.gz/zip download, Atom feed) — no per-ref archive UI on `/refs`. `archiveUrl`/`feedUrl` added
  to `lib/api/repos.ts` alongside the existing `rawUrl`, same link-only pattern (never `fetch`ed by
  the client). Hidden together with the rest of the empty-repository branch (`head === null`) —
  the feed endpoint would still return a valid empty feed, but hiding both is simpler than
  special-casing just the archive links.
- No new page/route, no `shellFor`/`shell_for` change. No API contract change — `docs/API.md`/
  `docs/openapi.json` untouched; `schemas.ts` gained `ReadmeInfo`/`ReadmeFormat` aliases,
  `lib/api/repos.ts` gained `getReadme`/`archiveUrl`/`feedUrl`.

## #22 `Dockerfile`: 3-stage alpine/musl build, `safe.directory = *`, pinned base images

Closes the deployment gap DECISIONS.md #4/#10 and ARCHITECTURE.md#Build/deploy described but never
implemented.

- **alpine/musl over debian-slim**: `git2` is `default-features = false` (no ssh/https transports),
  so there's no openssl-sys/libssh2-sys to fight with on musl. `libgit2-sys`'s build.rs finds no
  system libgit2 via pkg-config on alpine and falls back to building the vendored libgit2 source
  with `cc` — the only added builder package is `musl-dev`. The result is a fully static binary
  (musl targets default to `crt-static`), so the runtime stage needs no libgit2/zlib shared
  libraries at all. `utoipa-swagger-ui`'s `vendored` feature means its build.rs also never hits its
  download-a-zip code path — the whole build is network-free past `cargo fetch`/`pnpm install`.
  Confirmed with `ldd` on the built binary ("not a valid dynamic program").
- **Runtime packages are `git` + `ca-certificates` only** — no `tzdata`: `repo/meta.rs` only ever
  builds `Zoned` values from `TimeZone::UTC` or `TimeZone::fixed(offset)` (the offset recorded in
  the git commit itself), so jiff never consults the system tzdb.
- **`/etc/gitconfig` gets `[safe] directory = *`, plus a dedicated non-root user (uid 10001)**.
  `/srv/git` is a read-only bind mount owned by the git-server container's uid, which trips both
  `git`'s and libgit2's ownership check (both read the system gitconfig, so one file covers the
  `git archive`/`upload-pack` exec paths and the git2 `Repository::open` paths at once). Rejected
  alternatives: running as root (defeats the read-only, no-write-access posture CLAUDE.md's `Core
  invariants` asks for, and would still need *some* ownership handling once dropped to a real
  user); calling `git2::opts::set_verify_owner_validation(false)` in `main.rs` (only covers
  libgit2, not the two `git` exec call sites, so it wouldn't actually remove the gitconfig need —
  just add a second mechanism next to it). Trusting every directory is scoped to this
  single-purpose, read-only container, so the usual "arbitrary directory trust" risk this setting
  exists to prevent doesn't apply here.
  - Verified end-to-end against `fixtures/repos` bind-mounted `:ro` into the container (uid 10001)
    — clone/`--depth 1`/fetch, push rejection (403), archive download, and every web route all
    worked. The specific *negative* control (removing `/etc/gitconfig` and confirming a dubious-
    ownership failure) couldn't be reproduced on macOS Docker Desktop — its virtiofs bind mount
    reports files as already owned by the accessing container's uid, so the ownership mismatch
    this setting exists for never actually occurs locally. On a real Linux docker host (the actual
    git-compose deployment target), a `:ro` bind mount preserves the host's numeric uid, so the
    mismatch — and the need for this setting — is real there.
- **BuildKit cache mounts** (`--mount=type=cache`) for `pnpm`'s store and cargo's registry +
  `api/target`, keyed by the default cache id (mount target path) so repeat builds skip
  re-fetching/re-compiling unchanged dependencies. Cache mount contents don't persist into the
  image layer, so the api stage's release binary is `cp`'d out to `/axgit` inside the same `RUN`
  that builds it, before the cache mount unmounts.
- **Base images pinned to a minor version** (`node:24.11-alpine3.22`, `rust:1.97-alpine3.22`,
  `alpine:3.22`), collected into `ARG`s at the top of the file — a floating tag like `node:24-alpine`
  would let the image quietly drift on every rebuild. Verified all three tags actually resolve via
  `docker manifest inspect` before use. `apk` packages aren't individually pinned (alpine's
  per-minor-version repository only receives patches, so the `alpine:3.22` pin already bounds
  them); pnpm is pinned exactly by the root `package.json`'s `packageManager` field, and Rust
  crates by `api/Cargo.lock` + `--locked`. Digest-pinning (`@sha256:...`) would be stricter still
  but is deferred — bumping the minor tags is expected to be a deliberate, separate commit either
  way. These `ARG` refs were later qualified with their registry (`docker.io/library/...`) and the
  file itself renamed `Dockerfile` → `Containerfile` (`Dockerfile` kept as a symlink) — see #80.

## #23 3-way theme selector (System / Light / Dark)

Made the `.dark` token block live: it has been in `global.css` since #19, added for Shiki's
dual-theme tokens, with nothing in the app ever applying the class.

- **Two `is:inline` scripts, no client-only React logic for the resolve itself.** The pre-paint
  half must be a synchronous classic script in `<head>` (`Layout.astro`) — a bundled `<script>` is
  deferred (`type="module"`) and flashes the light theme on every load, the same lesson #17
  recorded for `RepoNav.astro`'s fill-in script. `<html data-theme>` carries the stored
  *preference* (`system`/`light`/`dark`); the `dark` class carries the *resolved* value, which is
  what `@custom-variant dark` and the `.dark {}` token block key off. Neither is ever rendered
  server-side: the shells are prerendered once per route shape and served to every visitor with
  `Cache-Control: no-cache` (#17), so a cookie-driven, server-rendered theme is structurally
  unavailable here — the mechanism has to be client-side.
- **The toggle control is a `ThemeToggle` React island (`client:only="react"`, with a static
  `slot="fallback"`)**, following the existing `RepoList` pattern rather than more inline script.
  Its trigger renders all three System/Light/Dark icons unconditionally; `global.css` reveals only
  the one matching `<html data-theme>`, so the correct icon already shows in the prerendered
  fallback, before the island hydrates. The icons are Phosphor React components — `iconLibrary:
  "phosphor"` in `components.json` had been declared but unused until now.
- **shadcn's `dropdown-menu` (Base UI `Menu`) with a `RadioGroup`/`RadioItem` for the three
  options**, generated via `shadcn add dropdown-menu` and reformatted with `prettier` to match the
  repo's style (no new runtime dependency — `@base-ui/react` was already installed for
  `ui/button.tsx`). `MenuRadioItem` renders `role="menuitemradio"` with `aria-checked` natively, so
  no custom ARIA wiring was needed. Selecting an item does not close the menu (Base UI's/Radix's
  documented default for radio items), which doubles as a quick way to preview each theme.
- **`global.css`**: `color-scheme: light`/`dark` on `:root`/`.dark` (scrollbars, native form
  controls, the canvas) keyed to the resolved class rather than `prefers-color-scheme`, so a manual
  override reaches native UI too; `@custom-variant dark` widened from shadcn's `&:is(.dark *)` to
  `&:is(.dark, .dark *)` — the class lands on `<html>` itself, which the descendant-only form
  excludes, and `@layer base` already styles `html`; and a small rule that shows only the
  `data-theme-icon` matching `<html data-theme>`, so the trigger's icon is correct at first paint
  with no client-side branching. No transition-suppression rule was added: the only
  `transition-colors` on a rendered element is `ui/table.tsx`'s row, whose background is
  transparent outside `:hover`, so nothing visibly animates on a switch.
- **`.dark --primary` was raised from `oklch(0.432 0.095 166.913)` to `oklch(0.72 0.13 166)`.**
  shadcn's generated dark primary was *darker* than its light one — backwards for a dark theme, and
  invisible until now because `.dark` was inert. At the original value every `text-primary` link
  (`RepoSummary`, `ReadmeView`, `404.astro`), `RepoNav`'s `border-primary` active-tab underline and
  `CodeBlock`'s `target:bg-primary/10` line highlight sat under ~3:1 against `--background`. The
  block was already internally inconsistent about this: `--sidebar-primary` in dark is
  `oklch(0.696 0.17 162.48)`, in the same range as the new value. Verified visually across the
  repo list, summary, log, commit diff, tree, blob (Shiki), and blame views.
- **`CommitView.tsx`'s `bg-green-500/10`/`bg-red-500/10` diff-line backgrounds were the only place
  in the app needing a dark variant** (now `dark:bg-green-500/20`/`dark:bg-red-500/20`) —
  everything else was already on semantic tokens and flips for free. Shiki (#19) needed no change
  at all; this is the first time `.dark .shiki-code span { color: var(--shiki-dark) !important }`
  has ever been exercised, and it renders correctly.
- **Testing**: `web/e2e/theme.spec.ts` covers default-from-`colorScheme` emulation (both light and
  dark OS), an explicit choice overriding the OS preference, persistence across reload, live
  OS-change tracking while the preference is `system`, and the invariant that the served HTML's
  `<html>` tag carries no theme of its own. `web/tests/` (vitest browser mode) gained nothing: it
  renders bare React components with no Astro layout, and the theme-resolve logic can't be
  extracted into an importable module without bundling it into a deferred script — the exact flash
  the design exists to prevent — so the head script is only reachable from e2e, same as
  `RepoLayout.astro`'s fill-in script.
  - Two Playwright/CDP quirks surfaced and were worked around in the test file, not the app:
    `colorScheme` emulation updates `matchMedia(...).matches` but does not reliably dispatch the
    MediaQueryList `"change"` event, and each `matchMedia()` call mints a distinct object, so an
    externally-dispatched event doesn't reach a listener attached to a different instance. The
    OS-tracking test pins the query to one shared object via `addInitScript` before the app's own
    (unmodified) `matchMedia()` calls run, then dispatches on that object — exercising the same
    listener a real OS-level toggle would notify.
  - A screenshot-based no-flash assertion was rejected: there is no API that observes "the class
    was set before first paint" (anything queryable via `page.evaluate` already runs after it), and
    Playwright drives `astro dev`, where Vite injects the compiled stylesheet as an inline
    `<style>` block and produces a FOUC the static build doesn't have — a screenshot would measure
    a dev-server artifact. The structural equivalent is asserted instead: the theme script is a
    synchronous, non-deferred, `src`-less script in `<head>`.
- **Known limitation**: README images are arbitrary repository content, so a transparent PNG with
  dark artwork can be invisible in dark mode. The usual fix (authored `prefers-color-scheme`
  `<picture>` sources) is unavailable because `rehype-sanitize` with no `rehype-raw` strips it
  (#21).
- No new page/route, so no `shellFor`/`shell_for` change. No API contract change — `docs/API.md`,
  `docs/openapi.json`, and `web/src/lib/api/types.ts` are untouched, and `api/src` is not touched
  at all.

## #24 Client-side routing via Astro's `<ClientRouter />` (refines #17)

Added `<ClientRouter />` (`astro:transitions`) to `Layout.astro`, turning same-origin link clicks
into client-side `<body>` swaps with a short fade on `<main>`, instead of full page loads. #17's
decision to prerender one shell per route shape and delete the old client router (`lib/router.ts`)
stands unchanged — this is Astro's own router, not a return to hand-rolled client-side routing, and
no new page/route/API surface is involved.

- **Why SPA mode over native cross-document view transitions**: the CSS-only
  `@view-transition { navigation: auto }` approach still does a full page load per navigation (it
  only animates the browser's native MPA transition), so it wouldn't touch the actual cost this
  change targets — React runtime, Shiki's per-language dynamic import, and DiceBear all
  re-executing on every tab switch between Summary/Tree/Log/Refs, which is this app's primary
  navigation pattern.
- **`<main>` is not persisted; `<header>` is.** Every data island (`RepoSummary`, `TreeView`,
  `CommitLog`, …) is `client:only="react"` and reads `location` exactly once, at mount
  (`lib/repo-param.ts`). Persisting `<main>` across a same-shell navigation (e.g.
  `/{repo}/tree/src` → `/{repo}/tree/src/lib`) would keep the *old* island alive showing stale data
  instead of remounting against the new URL — `transition:persist` was deliberately left off it.
  `<header>` is identical on every page and holds only the `ThemeToggle` island, so persisting it
  avoids re-hydrating that island (and re-flashing its disabled `ThemeToggleFallback` skeleton) on
  every navigation.
- **The theme resets on every swap unless explicitly reapplied.** Astro's swap
  (`swapRootAttributes` in its transitions runtime) replaces `<html>`'s entire attribute set with
  the incoming document's — and per #23, the prerendered shell's `<html>` carries neither
  `data-theme` nor `class` (`theme.spec.ts` asserts that absence directly), so a same-shell
  navigation would silently revert every page to light. `Layout.astro`'s theme script was
  refactored into a named `applyStoredTheme()` function, called once on initial load and again on
  every `astro:after-swap`.
- **`astro:after-swap`, not `data-astro-rerun`, for both shell-mutating scripts** (theme,
  `window.__axgit.fillRepoShell`). Astro's re-run mechanism (`data-astro-rerun`) executes inline
  scripts inside `runScripts()`, which only fires after the view transition's `updateCallbackDone`
  resolves — i.e. after the new page has already painted. `astro:after-swap` fires from inside the
  DOM-swap step itself, before that paint, which is the same "before paint" guarantee both scripts
  already relied on for the initial load. Verified by reading Astro 7.1.6's
  `node_modules/astro/dist/transitions/router.js` directly rather than assuming from the docs.
- **`RepoLayout.astro`'s fill-in script moved to `Layout.astro` as `window.__axgit.fillRepoShell`,
  keyed off a `[data-repo-name]` query rather than `document.currentScript`.** Two forced moves:
  (1) `document.currentScript` is `null` when Astro re-executes a script post-swap, so the old
  `document.currentScript.dataset.titleSuffix` read had to go — the suffix now travels as
  `data-title-suffix` on the heading element itself (`RepoNav.astro`, now taking `titleSuffix` as a
  prop). (2) The `astro:after-swap` listener has to be registered by a script present on *every*
  page, not just repository ones, so the very first navigation *into* a repository page already has
  a listener attached — `Layout.astro`'s head runs on every shell; `RepoLayout.astro`'s body script
  does not. `RepoLayout.astro` keeps a one-line `<script is:inline>window.__axgit.fillRepoShell();</script>`
  for its own initial load, which happens before `Layout`'s head script could otherwise discover
  the heading.
- **`prefetch.prefetchAll: false` in `astro.config.mjs`**, overriding the `true` default
  `<ClientRouter />` sets. Every `/{repo}/blob/*` (and tree/blame/commit) maps to one
  byte-identical shell served `Cache-Control: no-cache` (#17) — prefetching it on hover fetches the
  same HTML repeatedly for zero benefit, since the actual per-page data always arrives afterward
  via a separate `/api` call once the island mounts.
- **Dev middleware (`astro.config.mjs::shellFallback`) widened to recognize the router's own
  fetches.** It previously matched navigations by `Accept: text/html` only; `ClientRouter`'s
  `fetchHTML` sends no such header when there's no adapter (`Accept: */*`), which browsers pair
  with `Sec-Fetch-Dest: empty` — added as an alternate match so `astro dev` (and Playwright, which
  runs against it) keeps mapping `/{repo}/...` requests onto the placeholder-param shell instead of
  404ing every client-side navigation. Production (`api/src/shell.rs::serve_shell`) already matches
  on path shape alone, so it needed no change.
- **`::view-transition-old(root)`/`::view-transition-new(root)` animations disabled in
  `global.css`.** Without this, the root snapshot — covering the full viewport, including the
  persisted header and (on repository pages) the tab bar — would cross-fade underneath `<main>`'s
  own `transition:animate={fade(...)}`, doubling the animation on chrome that's supposed to swap
  instantly. No separate `prefers-reduced-motion` handling was added: `<ClientRouter />` already
  ships a media query that disables view-transition animation outright when it's set.
- **Testing**: `web/e2e/theme.spec.ts` gained a case asserting an explicit Dark choice survives a
  client-side tab navigation (the direct regression test for the reset behavior above) — the
  existing two structural assertions (no theme baked into the served shell; the theme script is a
  synchronous, non-deferred, `src`-less script in `<head>`) needed no change, since
  `data-theme-init`'s position and the shell's own markup are untouched. `web/e2e/repo.spec.ts`
  gained an assertion that a client-side marker set before navigating survives it, which is the
  only signal that a test is actually exercising client-side routing rather than a silently
  downgraded full reload landing on the same URL. No `web/tests/` (vitest) changes and no
  `shellFor`/`shell_for` change — no new route shape.
- No API contract change — `docs/API.md`, `docs/openapi.json`, and `web/src/lib/api/types.ts` are
  untouched, and `api/src` is not touched at all.

## #25 Repository list filter (client-side, `?q=`)

The first cut of #9's deferred "repository search" — a client-side filter over the repository list
`/` already fetches. Chosen as the "obviously in bounds" first step: `GET /api/v1/repos` already
returns `name`/`description`/`owner`/`section` for every repository, so no API change, no new
route/page, and `web/src/lib/shell.ts::shellFor` / `api/src/shell.rs::shell_for` are both untouched.

- **Matching rule** (`web/src/lib/repo-filter.ts::filterRepos`): the query is split on whitespace
  into terms; a repository matches only if *every* term (AND) is a case-insensitive substring of
  *at least one* of `name`/`description`/`owner`/`section`. A `null` field is skipped rather than
  matched or thrown on. An empty/whitespace-only query returns the input array unchanged — cheap
  and lets the caller detect "no filter active" by reference equality instead of a second flag.
  Filtering happens before `groupBySection`, so a section with no matches simply doesn't render.
- **`?q=` via `history.replaceState`, not `pushState`.** A history entry per keystroke would break
  the back button; `replaceState` still gives a shareable/bookmarkable/reload-safe URL. This
  doesn't interact with `<ClientRouter />` (#24) — the router only reads `location` at the moment
  of a link click, never observing the in-between states. The initial query is read once via the
  existing `lib/repo-param.ts::paramFromSearch` (same helper `CommitLog`/`TreeView`/`BlobView` use
  for `?ref=`/`?cursor=`/`?path=`), so a deep link lands already filtered.
- **`ui/input.tsx` added via `pnpm exec shadcn add input`** (vendored, same as every other
  `components/ui/` file) — the first shadcn component this project needed beyond what `dropdown-menu`
  (#23) and `table`/`skeleton` (#16) already brought in.
- No debounce: filtering an already-fetched in-memory array needs no async work to throttle.
- **Content search (file contents, commit messages) is explicitly not this** — it needs a new API
  endpoint and, per #9's original note, a real resource bound (`git grep`-per-request scans the
  whole tree every call) or an index, which would be this app's first piece of mutable persistent
  state in an otherwise stateless, read-only container (CLAUDE.md's core invariants). Left as the
  next ROADMAP.md item.

## #26 Repository search API: git2 in-process scan, not exec or an index

The second half of #9's deferred "repository search" (#25 was the client-side list filter) —
searching *inside* a repository: file content, file paths, and commit messages. `GET
/api/v1/repos/{repo}/search?q=&type=&ref=&limit=` (docs/API.md).

- **git2 in-process scan, not `git grep` exec, not a persistent index.** An index would be this
  app's first piece of mutable, persistent state in an otherwise stateless, read-only container —
  it changes the deployment shape (CLAUDE.md's core invariants) and was rejected outright. Between
  the two exec-free choices, git2 won over `git grep`: `cached_response` (`handlers/mod.rs`) is
  synchronous and git2 fits it directly, whereas an exec is inherently async and would force that
  helper into an async-closure shape for one caller. git2 also lets every scan enforce an *exact*
  byte/file/commit budget (below) instead of only a process timeout, and no query string ever
  reaches a command line. Same reasoning blame applied choosing git2 over exec (#14); if a large
  repository proves this too slow, a `git grep` exec fallback is the same escape hatch #14 left
  itself for blame.
- **One endpoint, `type=content|path|message`,** rather than three. All three share the "resolve a
  ref, walk something, cap the results" shape and the same response envelope (`sha`, `truncated`,
  `files`, `commits`) — splitting them into separate routes would triple the OpenAPI/shell
  boilerplate for no real gain, and cgit's own search UI presents them as one box with a type
  selector anyway.
- **Two independent budgets, not one.** `limit` (1–100, default 50, same rule as the commit log)
  caps *results*; a separate, fixed scan budget caps *work done regardless of matches* — 20,000
  tree entries walked, 32 MiB of blob content actually read (checked via `Odb::read_header` before
  a blob is loaded, so oversized/over-budget blobs are never pulled into memory), and 10,000
  commits walked for `type=message`. Either running out sets `truncated: true`. Content search
  reuses `repo/blob.rs::classify` (binary/1 MiB-cap detection) — the same classification blame and
  the blob endpoint already apply — so a search never surfaces something the blob view itself
  would refuse to render.
- **`q` is a fixed string, not a regex**, matched case-insensitively (ASCII fast path via
  `to_ascii_lowercase`, falling back to full `to_lowercase` for non-ASCII haystacks) — regex
  support would reopen the "unbounded worst-case cost from user input" problem the budgets above
  exist to close, for a feature cgit's own search doesn't offer either.
- Caching, immutability, and the empty-repository (unborn HEAD) carve-out all follow existing
  precedent exactly: commit-detail's "immutable only when `ref` is the resolved full sha" rule, and
  the commit log's "omitting `ref` on an empty repo is a 200 with empty results, an explicit `ref`
  is `404 ref_not_found`" rule.
- `handlers/commits.rs`'s `parse_limit`/`DEFAULT_LIMIT`/`MAX_LIMIT` moved to `handlers/mod.rs`
  (now `pub(crate)`) since search needed the identical rule — the second caller of the same logic.
  `repo/commits.rs`'s private `commit_info` was promoted to `pub(crate)` so `type=message` results
  reuse the exact log-entry shape (`CommitInfo`) rather than a parallel struct.
- The `/{repo}/search` web page is deferred to a follow-up commit (ROADMAP.md).

## #27 `/{repo}/search` page

The web UI for #26's search endpoint, closing out repository search (#9/#25/#26).

- **Route** `web/src/pages/[repo]/search.astro` follows `log.astro`'s shape exactly — no path
  segments, all state in the query string (`?q=&type=&ref=`) — needing only a two-segment match in
  `shellFor`/`shell_for` (`[_repo, "search"]`), unlike tree/blob/blame's unbounded-depth matching
  (#19/#20). `RepoNav.astro` gained a "Search" tab; `RepoLayout.astro`'s `Active` union/
  `TITLE_SUFFIXES`/`NAV_ACTIVE` grew a `search` case (it's its own tab, not a drill-down like
  commit/blob/blame).
- **Plain `<form method="get">`, no controlled inputs, no client-side state.** Reading Astro
  7.1.6's `ClientRouter.astro` source directly (the same verification standard #24 used) confirmed
  it registers a `submit` listener that intercepts same-origin GET forms exactly like it does
  anchor clicks, converting the submission into a client-side navigation to
  `?q=...&type=...&ref=...` — so the form needed no `onSubmit` handler, and degrades to an
  ordinary full-page GET if JS is unavailable. `q`/`type` are read via `defaultValue` (uncontrolled)
  and `paramFromSearch`, matching every other page's "props override, `location` is the default
  source" pattern (`CommitLog`, `TreeView`).
- **`SearchView`'s loading state is lazily derived from the initial query**, not reset inside the
  fetch effect — mirrors `CommitLog`'s existing rationale (a live-instance prop change only happens
  in tests; production always remounts fresh on a new URL) and avoids
  `@eslint-react/set-state-in-effect` warnings from a synchronous `setState` at the top of the
  effect.
- Result rendering branches on the response's `type`: `content`/`path` group by file
  (`repo-href.ts`'s new `blobLineHref` appends `#L{n}`, reusing `CodeBlock.tsx`'s existing anchor
  scheme from #19 — no new anchor mechanism needed) `message` reuses `CommitLog`'s row shape
  (avatar, summary link, short sha, relative time). A `truncated: true` response renders a visible
  banner — otherwise a capped result would look identical to a complete one.
- `web/src/lib/api/repos.ts` gained `searchRepo`/`SearchParams` (reusing the existing `buildQuery`
  helper); `repo-href.ts` gained `blobLineHref`/`searchHref`; `schemas.ts` gained
  `SearchResults`/`SearchKind`/`FileMatch`/`LineMatch` aliases. No API contract change.

## #28 Commit-statistics API: git2 revwalk, 12 fixed buckets anchored on the resolved commit

`GET /api/v1/repos/{repo}/stats?ref=&period=&limit=` (docs/API.md) — cgit's `stats` page, the
other half of #9's v1 exclusions (repository search, #25/#26/#27, was the first). Closes out #9.

- **git2 in-process revwalk, not a `git log` exec, not a persistent index.** Same reasoning as
  search (#26): an index would be this app's first piece of mutable, persistent state in an
  otherwise stateless, read-only container, and an exec can only be bounded by a process timeout
  where git2 lets the walk enforce an exact commit budget (`MAX_SCANNED_COMMITS = 20_000`,
  the same budget family as search/archive/diff/blob's existing caps).
- **The window is anchored on the resolved commit's authordate, not the request time.** cgit's own
  stats page anchors on "now," but doing that here would make every response time-dependent —
  ineligible for the immutable full-sha caching rule every other endpoint gets
  (`handlers/mod.rs::cached_response`). Anchoring on the commit instead makes a full-sha `ref`
  request fully deterministic, so it gets the same `Cache-Control: public, immutable` treatment as
  commit detail/diff/blame.
- **12 buckets, fixed, regardless of `period`.** Keeps the response shape (and the web table's
  column count) independent of the period selection — `period` only changes each bucket's
  duration (week/month/quarter/year), not how many there are.
- **Bucket boundaries are calendar-aware, computed with jiff's `Span` (not fixed-duration
  arithmetic).** Month/quarter/year lengths vary, so subtracting "1 month" from a `Date` (jiff
  handles this correctly, unlike naively subtracting a fixed number of seconds) is required for
  bucket starts to land on the 1st. Week buckets start on Monday (`Weekday::to_monday_zero_offset`).
  All bucketing happens in UTC — the anchor's original offset (the commit's authored timezone) is
  discarded once used to pick which UTC instant to anchor on, so the response's bucket boundaries
  don't depend on where the author's clock was set.
- **A commit authored after the window's end clamps into the last bucket rather than being
  dropped** — out-of-order authordates are possible across merged branches (an old branch merged
  late). Commits older than the window are simply excluded; no early-exit heuristic stops the
  revwalk on the first out-of-window commit, since revwalk order isn't strictly chronological
  across merges — only the scan budget bounds the walk, same as search's message search.
- **`authors[].buckets` is a parallel array to the top-level `buckets`**, avoiding a second
  boundary/label scheme the frontend would have to keep in sync — same shape idea as feed reusing
  the commit log's `CommitInfo`.
- **`limit` (default 50, 1–100, not clamped) caps only the `authors` rows returned, never the
  bucket totals** — `buckets[].commits` always reflects every commit in the window regardless of
  which authors got cut, and `author_count` reports the full distinct count so the UI can show
  "top N of M." `truncated` is set by either cause (scan budget or `limit`), matching search's
  precedent of one flag for two independent causes.
- `handlers/mod.rs::parse_limit` gained a third caller (after commits/search); `repo/commits.rs`'s
  `signature_info`/`CommitAuthor` are reused directly for the author breakdown (email is never
  exposed, only its hash, per the existing rule).
- The `/{repo}/stats` web page (chart + author table) is deferred to a follow-up commit
  (ROADMAP.md), same split search used (#26 API → #27 page).

## #29 `/{repo}/stats` page: Recharts (via shadcn), dataviz-validated chart color

The web UI for #28's endpoint — closes out #9's v1 exclusion list (the last item; HTTP push
remains permanently excluded by the read-only invariant, not deferred).

- **Route** `web/src/pages/[repo]/stats.astro` follows `search.astro`'s shape (no path segments,
  all state in `?period=&ref=`) — one new two-segment case in `shellFor`/`shell_for`
  (`[_repo, "stats"]`). `RepoNav.astro` gained a "Stats" tab; `RepoLayout.astro`'s `Active`/
  `TITLE_SUFFIXES`/`NAV_ACTIVE` grew a `stats` case (its own tab, not a drill-down). Since
  `/{repo}/stats` was the 404-shell test fixture `theme.spec.ts` and `repo.spec.ts` deliberately
  relied on (a route with zero API-dependent islands, needed to test the theme toggle/404 page in
  isolation), both moved to `/{repo}/blob` (no path segment — still a structurally unmatched
  shape) instead.
- **Chart library: Recharts, installed via `pnpm exec shadcn add chart`**, not `pnpm add
  recharts` directly — the `base-luma` style's `chart` registry item declares `recharts` as its
  own dependency (the CLI installs the exact pinned version) and pulls in `registryDependencies:
  card`, so `ui/chart.tsx` **and** `ui/card.tsx` (unused so far, vendored regardless — registry
  dependencies aren't cherry-picked) both land the same way every other `ui/` file does. The
  vendored `chart.tsx` had one lint error (`@typescript-eslint/consistent-type-definitions` on
  its `ChartConfig` type alias) fixed via `eslint --fix`; its remaining warnings (React 19
  `useContext`/`Context.Provider` style, `dangerouslySetInnerHTML`, two array-index keys) are
  left as shadcn generated them, same "vendored, modify only what's broken" policy CLAUDE.md
  states — none of them are errors that fail `pnpm check`.
- **`--chart-1` was broken and got fixed, following the `dataviz` skill's procedure instead of
  eyeballing it.** Like `--primary` before #23, `global.css`'s `--chart-1..5` had never been
  exercised (no chart existed yet) and its light/dark values were byte-identical shadcn
  boilerplate. Running the skill's `validate_palette.js` against the converted hex confirmed real
  breakage: light mode read at **1.44:1** contrast against the surface (FAIL, essentially
  invisible as a bar fill) and dark mode's lightness (OKLCH L 0.855) sat above the dark band
  (0.48–0.67, FAIL — would read as glowing). Fixed by reusing `--primary`'s hue/chroma (165.612 /
  0.118) at two different lightness steps: `--primary`'s own light value (L 0.508, already
  validator-PASS) for the light-mode chart-1, and a new L 0.6 step for dark mode — `--primary`'s
  own dark value (L 0.72) was tuned for *text* contrast, not a chart mark's dark-band requirement,
  so reusing it as-is would have repeated the same FAIL. `--chart-2..5` are untouched and
  unvalidated — nothing uses them yet (this page has exactly one series); fix them when a second
  series exists. Chosen a single validated hue over introducing the skill's reference
  blue/orange/aqua palette, since the app already has an established brand hue and this chart has
  no multi-series adjacency requirement to justify a bigger palette.
- **`minPointSize={2}` on the `<Bar>` — a real bug the skill's "render it and look at it" step
  caught.** Without it, Recharts omits the rectangle element entirely for a zero-value bucket (a
  month with no commits) — confirmed by inspecting the rendered SVG (11 bar elements for 12
  buckets). No element means no hover/tooltip hit target for that bucket, violating
  `references/interaction.md`'s "the mark is the hit target" rule. `minPointSize` keeps a thin
  visible/hoverable sliver instead. (The same render-and-look pass also caught, and ruled out, a
  false alarm: an early screenshot showed near-invisible slivers for *every* bar — that was
  Recharts' entrance animation caught mid-flight by too short a wait in the ad-hoc screenshot
  script, not a real rendering bug; the settled render confirmed bar heights are correctly
  proportional.)
- **Period switcher is four plain links (`statsHref`), not a form** — unlike `SearchView`'s free-
  text query, the period is a fixed 4-way pick, so a `<form>` (needed there for the no-JS
  fallback) adds nothing; `<ClientRouter />` (#24) already intercepts the link clicks.
- **The author table doubles as the chart's required "table view"** (`references/color-formula.md`
  / accessibility-pass rule: every value shown by a mark must be reachable without it).
  `authors[].buckets` (parallel to the response's `buckets`, #28) becomes one table column per
  bucket, and a `TableFooter` "Total" row sums each column — the same numbers the chart plots, so
  nothing is chart-only. `author_count` vs `authors.length` renders as a "Showing top N of M"
  note when `limit` cut the list (search's `truncated` banner precedent, reused here for the
  scan/limit truncation flag too).
- `web/src/lib/api/repos.ts` gained `getStats`/`StatsParams` (reusing the existing `buildQuery`
  helper); `repo-href.ts` gained `statsHref`; `schemas.ts` gained the stats type aliases. No API
  contract change — this commit is web-only.

## #30 Lazy-load Recharts, react-markdown, and the theme menu behind `React.lazy`

ROADMAP.md's "web build's largest JS chunk is over the 500 kB warning threshold" candidate,
picked up on its own. The premise on file turned out to be wrong once measured: the chunk that
actually trips Vite's 500 kB warning is Shiki's `cpp` grammar (637 KB) — already lazy, loaded
only when a `.cpp` file is opened, and left untouched here. The real problem was three chunks
that load *eagerly*, one of them (`ThemeToggle`) on every single page:

| Chunk | Before | Loads on |
|---|---|---|
| `ThemeToggle` | 137 KB | every page |
| `StatsView` | 345 KB | `/{repo}/stats`, unconditionally |
| `ReadmeView` | 145 KB | `/{repo}`, even with no README |

- **`ThemeToggle`/`ThemeMenu`/`theme.tsx` three-way split** — `@base-ui/react`'s Menu (floating-ui
  positioning, ~121 KB) was the only thing in the app pulling in `@base-ui/react` at all
  (`grep -rln "ui/button" src/` found only `ThemeToggle.tsx`), so it was a fully exclusive chunk
  and moving its import behind `import()` moved the bytes outright. `theme.tsx` holds the leaf
  state/icons shared by both `ThemeToggle.tsx` (eager) and the new `ThemeMenu.tsx` (lazy) —
  neither of the two imports the other, since a cross-import would pull the lazy side's weight
  back into the eager chunk.
  - Chose base-ui's **`defaultOpen`** over controlled `open`/`onOpenChange`: `useInitialOpenSync`
    (`@base-ui/react`'s internal store) initializes the popup open on first render and then owns
    every close path (Escape, outside-press, trigger re-click, scroll lock) itself — nothing to
    resync from the eager side. Verified by reading `@base-ui/react` 1.6.0's source directly
    (`MenuRoot.d.ts`, `utils/popups/popupStoreUtils.js`): a trigger mounting in the same commit as
    an already-open popup is claimed pre-paint by `registerTrigger`/`useImplicitActiveTrigger`, so
    there's no anchor-position gap, and `FloatingFocusManager`'s `initialFocus: true` moves focus
    into the popup regardless of `openMethod` being `null` for a programmatic open. Manually
    verified end-to-end in a running dev server (not just the automated suite) since this is
    exactly the kind of interaction a component test can't reach: Tab-to-focus + Enter opens the
    real menu with focus landing inside the popup container, and click-based arrow-key selection
    updates `aria-checked`/`data-theme`/`localStorage` while the popup stays open (`closeOnClick`
    defaults to `false` on `MenuRadioItem`, matching #23's already-tested "selecting doesn't close
    the menu" behavior — not re-implemented here).
  - The eager trigger's eventual `onClick` wraps `setMenuRequested(true)` in `startTransition` —
    `@astrojs/react`'s own `startTransition` only covers the initial `client:only` mount, not a
    later click that causes a boundary to suspend; without it a synchronous update that suspends
    commits the Suspense fallback and warns in dev. `onPointerEnter`/`onFocus` also warm the
    dynamic import ahead of an actual click, and the plain pre-interaction button and the
    `Suspense` fallback are the literal same `PlainTrigger` element, so there is nothing to flash
    even if the fallback ever actually commits.
  - Result: `ThemeToggle`'s entry chunk 137 KB → ~2 KB; new `ThemeMenu` chunk ~119 KB, fetched
    once, on first hover/focus/click.
- **`StatsView`/`StatsChart` split**, with **pre-warming** — `StatsChart.tsx` now owns
  `recharts` + `ui/chart` + `chartConfig`; `StatsView.tsx` fires
  `void import("./StatsChart")` in parallel with its `getStats` fetch, since the stats page
  renders a chart for nearly every response — serializing the chunk fetch behind the API round
  trip would be pure latency with no payoff. `Suspense`'s fallback is `<Skeleton className="h-64
  w-full" />`, pixel-matched to both `ChartContainer`'s own sizing and `StatsViewSkeleton`'s
  middle block, so there's no layout shift regardless of which resolves first.
  Result: `StatsView` 345 KB → ~5 KB; new `StatsChart` chunk ~333 KB.
- **`ReadmeView`/`ReadmeMarkdown` split**, deliberately **not** pre-warmed — the opposite policy
  from `StatsChart`, on purpose: a repository with no README, or one whose README is
  `rst`/`plain`, should never download react-markdown/remark-gfm/rehype-sanitize at all, and
  `format` isn't known until the `/readme` response lands. `ReadmeMarkdown.tsx` took the entire
  markdown-only surface — `InlineCode`, `textContent`, `MarkdownFence`, `rewriteHref`/
  `rewriteSrc`, and the `components` map — as one unit, keeping `InlineCode` and the `pre`
  override's `child.type === InlineCode` identity check (#21) in the same module; splitting them
  across files would have reintroduced that exact bug with no type error to catch it a second
  time. `components` is now also wrapped in `useMemo` keyed on `repo` — a pre-existing (harmless
  but wasteful) inefficiency where every `ReadmeView` re-render rebuilt the map and remounted the
  whole markdown tree, caught while doing this split, not a new defect.
  Result: `ReadmeView` 145 KB → ~2 KB; new `ReadmeMarkdown` chunk ~140 KB, fetched only for an
  actual markdown README.
- **Confirmed non-issues, left alone**: `ui/dropdown-menu.tsx`'s `@phosphor-icons/react` barrel
  import already tree-shakes correctly (only the icons actually used land in the chunk — checked
  by grepping the built output); Vite's chunkSizeWarningLimit and the `cpp` grammar chunk are
  unchanged, since that chunk is already behind `lib/format/highlight.ts`'s existing lazy-language
  loading and only downloads when a `.cpp`/`.hpp` file is actually viewed.
- All three splits follow the one dynamic-import precedent already in the codebase
  (`lib/format/highlight.ts`'s Shiki language loaders) — confirmed rolldown treats a dynamic
  `import()` as a real chunk boundary here (`cpp`/`markdown`/`core` were already separate chunks
  before this change), not something that gets hoisted back into the static graph.
- No API contract change, no new route — `docs/API.md`/`docs/openapi.json`/
  `web/src/lib/api/types.ts`/`shellFor`/`shell_for` all untouched.

## #31 `/{repo}` summary: two-column layout, description + metadata in a right sidebar

Restyled the Summary page GitHub-style: the README becomes the wide main column, and the
description + metadata `<dl>` (previously a full-width block above the README) moves into a
narrow right sidebar.

- **Grid lives in `pages/[repo]/index.astro`, not in either island.** `RepoSummary` and
  `ReadmeView` stay two independent `client:only="react"` islands (unlike #30's `ReadmeView`/
  `ReadmeMarkdown` split, which merged the markdown surface *within* one island). With
  `client:only`, Astro emits `<astro-island>{fallback}</astro-island>` and React replaces the
  children in place — keeping the two column boxes as plain Astro elements around the islands
  means the geometry is identical before, during, and after hydration, so the `slot="fallback"`
  skeletons never snap from full-width into a sidebar. The alternative (owning the grid inside a
  merged React component) would require one combined fetch/skeleton and would pull
  `ReadmeMarkdown`'s module graph into the summary chunk regardless of README format —
  defeating #30's lazy split.
- **`lg:flex-row-reverse`, not `order-*` or `flex-col-reverse`.** DOM order is
  details-then-README at *every* breakpoint; only the desktop horizontal placement changes.
  Considered `flex-col-reverse` + `lg:flex-row` (visually equivalent) but rejected it: that
  would reorder the *mobile* DOM instead, forcing keyboard/screen-reader users through an
  arbitrarily long README before reaching the details block rendered visually above it — a
  focus-order/reading-order regression (WCAG 1.3.2/2.4.3) for the sake of matching desktop
  markup order that doesn't matter there. A `grid` + `lg:order-*` alternative would be
  equally correct but needs classes on three elements instead of one; not needed here.
- **`lg` (64rem) breakpoint, not `md`.** At `md` (48rem) the `max-w-5xl` content box is 736px;
  minus an 18rem (288px) sidebar and the gap, ~416px is left for the README — too narrow for
  code fences and GFM tables. `lg` leaves 672px, and 64rem is exactly `max-w-5xl`, so the
  two-column layout starts precisely where the container stops growing.
- **`min-w-0` on the README wrapper is required, not decoration.** Flex items default to
  `min-width: auto`; a `<pre>` or GFM table with a long unbroken line would otherwise set the
  item's min-content width and push the sidebar off-screen. Left unprefixed (applies in the
  mobile column too, where the same overflow risk exists).
- **`Layout.astro`'s `max-w-5xl` deliberately left unchanged**, even though a wider shell was
  briefly considered for more README room. The `<header>` (`Layout.astro:154`) shares the same
  width and is `transition:persist`ed (#24) — it is never re-rendered across a client-side
  navigation. A summary-page-only width would leave the content edge visibly offset from the
  (unmoving) header, and would visibly jump on every tab click. Widening is a global decision
  for both elements together, not a per-page one.
- **Branches/tags collapsed into one "Refs" `<dt>`** with two `MetaLink`s whose accessible
  names are the full phrase (`"3 branches"`, `"1 tag"`), both pointing at the new
  `repo-href.ts::refsHref`. A bare `<a>{count}</a>` would have an out-of-context accessible
  name ("3") — rejected on WCAG 2.4.4 grounds. No `#branches`/`#tags` fragment: `RefsView` is
  `client:only` and its sections don't exist in the DOM when the browser processes a fragment
  on initial load, so the scroll would silently fail.
- **`components/ui/card.tsx` still not used.** It was vendored only as a `shadcn add chart`
  registry dependency (#29) and remains unused; its `rounded-4xl`/`shadow-md`/`bg-card` look
  doesn't match this app's flat, hairline-border surfaces (`RepoNav`'s `border-b`, `<pre>`
  blocks). The sidebar's one `border-border border-t` rule under the description follows the
  existing idiom instead.
- Icons (`GitBranchIcon`/`TagIcon`/`DownloadSimpleIcon`/`RssIcon`, `@phosphor-icons/react`,
  same per-icon `dist/ssr/*` deep-import idiom as `theme.tsx`) sit only on the link rows, each
  with `aria-hidden="true"` — load-bearing, since `RepoSummary.test.tsx` and `repo.spec.ts`
  look the download/feed links up by accessible name and an un-hidden `<svg>` could otherwise
  affect it.
- `<aside aria-label="Repository details">` lives in the Astro page (not inside `RepoSummary`),
  so the `complementary` landmark exists in the prerendered HTML and survives the island's
  loading/error branches — the error branch is a bare `<p role="alert">` with no wrapper of its
  own. No new heading was added; the page's only headings remain `RepoNav`'s `<h1>` and
  `ReadmeView`'s path-label `<h2>`/markdown headings, so no existing `getByRole("heading")`
  lookup (including the "must stay unambiguous" fixture note in `e2e/repo.spec.ts`) is
  affected.
- No API contract change, no new route — `docs/API.md`/`docs/openapi.json`/
  `web/src/lib/api/types.ts`/`shellFor`/`shell_for` all untouched.

## #32 The page fade is a plain CSS animation, not a view transition (fixes a Firefox squash, refines #24)

In Firefox, every client-side navigation visibly compressed or stretched the page vertically for
the duration of the transition before snapping back to the correct size; Chromium was unaffected.
The cause was `<main>`'s `transition:animate={fade(...)}` (#24), so the fix removes the
View Transition API from the visual layer entirely: `<main>` no longer carries a
`view-transition-name`, and `global.css` animates the element itself.

- **Why it distorted.** Naming an element makes the UA animate its `::view-transition-group` box
  from the old element's size to the new one's, and paint the old/new snapshots inside that
  animating box. Firefox scales the snapshot to fit the box; Chromium keeps the snapshot at its own
  intrinsic block size (the spec's `block-size: auto` on `::view-transition-old/new`) and lets it
  overflow. Identical markup, opposite results — this is a rendering difference, not a bug in the
  app's CSS, which is why no duration/easing tweak would have helped.
- **`client:only` islands are what made it dramatic.** The group's target size is `<main>`'s height
  *at the DOM swap*, and at that instant `<main>` holds nothing but the static `slot="fallback"`
  skeletons (#17) — the real content lands milliseconds later, while the transition is still
  running. So the live content was being scaled into a box sized for a skeleton: squashed when the
  page turned out taller (the log), stretched when shorter. Measured on the built app with an
  ad-hoc Playwright script in both browsers, and confirmed visually in a *real* Firefox window
  against a fixed reference ruler placed outside `<main>` — headless Firefox does not reproduce the
  mis-scaled paint, so screenshots from a real window were required to see it.
- **Overriding the pseudo-elements was rejected** (`object-fit: none` / an explicit `block-size` on
  `::view-transition-old/new`). It would make the fade's correctness depend on Firefox honoring an
  override for sizing it already renders differently from the spec's default — an unverifiable
  bet — where dropping the name removes the failure mode outright.
- **`::view-transition-group(root)` is now disabled too**, alongside the `old`/`new` rules #24
  already had. With no named elements left, the page is a single `root` snapshot whose size cannot
  change (it is always the viewport-sized snapshot containing block), so the group animation is a
  250 ms no-op that only keeps a frozen snapshot on screen. Off, the transition resolves within a
  frame and the real DOM is back immediately — `<ClientRouter />` still drives the swap through
  `startViewTransition` (that is what keeps `astro:after-swap` firing before paint, which both
  head scripts depend on), it just no longer animates anything.
- **What changed visually**: the incoming page fades in over 0.18 s (`@keyframes axgit-page-in`);
  the outgoing one is simply gone once the swap commits, where before the two cross-faded. `<main>`
  is never `transition:persist`ed (#24), so Astro inserts a fresh element on every navigation and
  the animation restarts on its own — no class toggle, no `astro:after-swap` hook. It also plays
  once on a cold load, which the shared-element version could not do.
- **`prefers-reduced-motion` now needs its own rule.** #24 relied on `<ClientRouter />`'s built-in
  media query, which only governs *view transition* animations; a plain CSS animation isn't covered
  by it, so `global.css` disables it explicitly.
- **Testing**: no test changed. Nothing asserted the fade (only that navigation is client-side —
  `repo.spec.ts`/`stats.spec.ts`'s `window`-survives-navigation checks), and the full suite passes
  unchanged: 134 vitest, 20 Playwright e2e, `pnpm check`. The fix itself was verified with a
  throwaway script asserting both browsers now report **zero** `::view-transition-*` animations
  during a navigation and an identical `axgit-page-in` opacity ramp on `<main>`. No new
  route/shell (`shellFor`/`shell_for` untouched) and no API contract change — this commit is
  web-only.

## #33 Commit graph column laid out client-side, no walk-order change

The Log tab gained a leading graph column (cgit's ASCII DAG, redone as inline SVG). It needed
no API change: `CommitInfo.parents` (full shas, all parents, git parent order) has been in every
`/commits` response since the endpoint was built, and no frontend code had read it before this.

- **The walk stays unsorted, deliberately** (`api/src/repo/commits.rs::log`). A "correct" DAG
  layout wants topological order (what `git log --graph` implies via `--topo-order`), but
  libgit2 1.9.6's `revwalk.c` shows any sort flag (`GIT_SORT_TOPOLOGICAL`, or even bare
  `GIT_SORT_TIME`) sets `walk->limited = 1`, which makes `prepare_walk` run `limit_list` —
  draining and parsing the *entire reachable history* before the first commit is emitted.
  `GIT_SORT_NONE` (today's setting) keeps `walk->limited = 0`, a lazy O(page size) walk. Since
  the response cache keys on the cursor, N pages on a cold cache would mean N full-history walks
  under any sort flag. Not worth it: plain committer-date order is enough for a correct graph, because
  within a single page a parent can never be *emitted* above its child (libgit2's lazy walk only
  discovers a commit by popping an already-emitted child) — the only real cost is that branches
  interleave more than `--topo-order` would, so lanes run a bit longer and cross more.
- **One row per commit, no cgit-style `|\`/`|/` filler rows.** Filler rows would leave the other
  four columns empty and break `TableRow` hover/zebra/row semantics on a shadcn table. The
  trade-off: a merge edge gets only half a row (24px) of vertical travel, so a wide lane jump
  reads as near-horizontal. Leftmost-free lane allocation (`web/src/lib/commit-graph.ts`) keeps
  most jumps to 1–2 lanes, and lanes never shift horizontally once assigned (a freed lane is left
  as a hole and reused later) — every `through` line is a straight vertical, and the only
  diagonals are a node's own half-edges into/out of its row.
- **Monochrome, merge-vs-normal encoded as shape (hollow ring vs filled dot), not colour.**
  `--chart-2..5` are unvalidated shadcn boilerplate (#29) that the roadmap wants run through the
  dataviz skill's validator before a second series uses them — this sidesteps that question
  entirely rather than deferring it. Do not "improve" this with per-lane hue without running that
  validator first.
- **Hidden when `?path=` is set.** `touches_path` yields a subsequence of the true history —
  displayed commits are usually not each other's parents — so edges would be arbitrary.
  Also hidden under `sm` (`hidden sm:table-cell`) so it can't crowd the useful columns off a
  narrow, already-`overflow-x-auto` table.
- **Lanes reset at each page boundary.** The cursor is an opaque commit sha (`docs/API.md`),
  validated as a git2 `Oid` — encoding lane state into it would break that contract, and cgit has
  the same per-page reset. `continuesAbove` (set whenever the page was reached via `cursor`) draws
  row 0's own edge to the top rather than showing a fake root when its children are actually on
  the previous page.
- **No new dependency.** The layout + renderer is ~190 lines / a few KB, small enough that — unlike
  `StatsChart.tsx`'s recharts (#29/#30) — it does not need lazy-loading.
- **Ref badges (HEAD/main/v1.0.x) deliberately left out of this pass.** #18 already rejected
  prefetching `/refs` for a page that doesn't otherwise need the request; if badges are wanted
  later they belong in the Summary cell as separate work.
- **Pre-existing issue this makes visible, not introduced by it**: page ≥2 pushes only the cursor
  commit, so a side-branch commit pending at the page cut that isn't an ancestor of the cursor
  silently drops out of every later page. Noted as a `docs/ROADMAP.md` candidate.
- No API contract change — `docs/API.md`/`docs/openapi.json`/`web/src/lib/api/types.ts`/`api/**`
  all untouched.

## #34 Ref badges on the Log tab and commit detail, via a parallel client-side `/refs` fetch

Closes the one item #33 deliberately deferred: decorating a commit with the branch/tag names that
point at it (cgit's ref decoration). Each commit row (and the commit detail header) now shows a
badge per branch/tag whose tip is that commit, linking to `/{repo}/log?ref={name}`.

- **Not the prefetch #18 rejected.** #18 rejected reimplementing the api's ref longest-match
  client-side to *resolve a URL* — a blocking dependency that would gate rendering on a second
  request. Badges are pure decoration: `web/src/lib/commit-refs.ts::useCommitRefs` fires `/refs`
  in its own effect, parallel to and independent of the page's own commits/detail fetch. Zero API
  change — `CommitInfo` (shared with `/search?type=message`) is untouched.
  `indexRefsBySha`/`useCommitRefs` are unit-tested separately (`web/tests/lib/commit-refs.test.ts`)
  from the two components that consume the hook.
- **Fails silently, same as `ReadmeView`'s 404 (#21).** A rejected `/refs` request just leaves the
  index empty — no error state, no retry, the commit list/detail renders exactly as it would
  without it. Ref badges are additive polish, not required data.
- **No HEAD/default-branch badge.** `GET /refs` returns branches and tags only, and
  `default_branch` lives on the repo summary — pulling in a third request for a styling nuance
  wasn't worth it. The default branch still appears, just as an ordinary branch badge (e.g. `main`
  next to its tip commit), not specially marked.
- **Icon distinguishes kind, not colour** — `GitBranchIcon`/`TagIcon` (already used this way on
  `RepoSummary.tsx`). Same reasoning as #33: `--chart-2..5` are still unvalidated shadcn
  boilerplate (#29), so nothing new gets colour ahead of the dataviz skill's validator.
- **Capped at 3 in the log table, uncapped in the commit detail header.** A heavily-tagged commit
  must not blow out the `h-12` row height #33 relies on for the graph column; the table shows the
  first 3 refs (branches before tags, both already name-sorted by the api) plus a `+N` link to
  `/{repo}/refs`. The commit detail page has room and shows every ref.
- **New `web/src/components/ui/badge.tsx`** (`pnpm exec shadcn add badge`, vendored like
  `input`/`chart` in #25/#29) — `variant="secondary"` for branches, `"outline"` for tags, wrapped
  in an `<a>` via its Base UI `render` prop (same pattern as `ThemeMenu.tsx`'s
  `DropdownMenuTrigger`). `logHref` moved from `CommitLog.tsx` into `web/src/lib/repo-href.ts`
  (matching `searchHref`/`statsHref`'s shape) so the new `RefBadges.tsx` and `CommitLog.tsx` can
  both build ref links from it.
- No API contract change — `docs/API.md`/`docs/openapi.json`/`web/src/lib/api/types.ts`/`api/**`
  all untouched. No new route, so `shellFor`/`shell_for`/`RepoNav.astro` are untouched.

## #35 cgit URL compatibility redirects

Jenkins' `git-plugin` Repository browser was still set to `cgit`, so build "changes" links were
rendered as `/{repo}.git/commit/?id={sha}` — a shape axgit has never served. Separately, any
`.git`-suffixed page URL (old cgit bookmarks, wiki links) silently rendered a broken page: it
matched `shell::shell_for`'s route-*shape* matching (which never validates the repo segment), so
the shell served 200, then the client-side island resolved the repo as literally `{repo}.git` and
the API 404'd.

- **Two-part fix.** (1) Jenkins' job Repository browser was switched to `githubweb` (URL without a
  `.git` suffix) — its changeset (`{url}/commit/{sha}`), file (`{url}/blob/{sha}/{path}`), and diff
  (`{url}/commit/{sha}#diff-N`) link shapes match axgit's native routes exactly, so this alone needs
  no axgit change. (2) A permanent-redirect compatibility layer for the cases that already existed
  before the Jenkins change: old cgit URLs and any `.git`-suffixed page request.
- **Core shapes only, not full cgit coverage.** `/{repo}[.git]/commit/?id=` and `/{repo}[.git]/diff/`
  → `/{repo}/commit/{sha}`; `/{repo}[.git]/log/?h=` → `/{repo}/log?ref=`; a bare `/{repo}.git` →
  `/{repo}`; any other `.git`-suffixed path has the suffix stripped, query preserved. cgit's
  `tree/{path}?id=` is **not** split into `tree` vs `blob` — telling those apart needs a git lookup
  the redirect layer doesn't have — so it only gets the generic `.git`-strip. `plain/`, `atom/`,
  `snapshot/` are out of scope; nothing currently links to them.
  cgit-shaped queries (`commit`/`diff`/`log` above) redirect **regardless of `.git`**, since
  `/{repo}/commit` (2 segments) was never a valid shape either way. Anything that's already a valid
  native shape is left untouched — this is also what rules out redirect loops: the new
  `cgit_compat::redirect_for`/`redirectFor` return `None`/`null` whenever the computed target would
  equal the request as-is.
- **New `api/src/cgit_compat.rs`, wired into `shell::serve_shell_or_redirect`** (replaces
  `serve_shell` as the static-fallback handler in `routes.rs`; `serve_shell` itself is unchanged and
  still exported for the shapes that fall through). Runs after `ServeDir` and the Smart HTTP /
  Swagger UI routes (both real routes on the router, matched before the fallback ever executes), so
  clone/fetch and the API are untouched — verified by
  `cgit_compat_test.rs::smart_http_routes_take_precedence_over_cgit_redirects` and
  `::dot_git_clone_still_works`.
- **No percent-decoding.** `id`/`h` are matched and copied into the `Location` on their raw,
  still-encoded text; `id` is additionally constrained to `[0-9a-fA-F]{4,64}` before it's accepted.
  Together this means nothing built here can carry characters the original request didn't already
  contain — no decode/re-encode round trip, no header-injection surface.
- **Mirrored into the dev server**, same as `shellFor`/`shell_for`: new `web/src/lib/cgit-compat.ts`
  (`redirectFor`), called from `astro.config.mjs`'s `shellFallback()` middleware ahead of the
  `shellFor` rewrite, answering a real `308` instead of rewriting `req.url`.
- No API contract change — `docs/API.md`/`docs/openapi.json`/`web/src/lib/api/types.ts`/`api/**`
  (beyond the new module) all untouched; this is static-fallback behavior, not an `/api/v1` route.

## #36 blame rename tracking (corrects #14's premise)

ROADMAP.md had carried "`git blame --follow` (rename tracking)" as an unrevisited candidate since
blame was built (#14/#20), on the assumption that libgit2's default blame has no rename tracking
at all.

- **That assumption was wrong.** libgit2's `blame_git.c::find_origin` runs its own rename-detecting
  diff (`git_diff_find_similar` with `GIT_DIFF_FIND_RENAMES`) between a commit and each parent
  while walking blame — the same default-threshold similarity match `git blame` itself uses. This
  was verified directly: a small C program linked against the system's libgit2 1.9.6 was compared
  against `git blame --porcelain` across three histories (plain rename, rename + edit in the same
  commit — including a move into a subdirectory, and a multi-hop rename chain a→b→c). All three
  matched line-for-line, including which commit each range attributes to and the `orig_path`
  chain. So no exec fallback was needed to get rename tracking working — it already worked; it just
  wasn't surfaced in the response.
- **What's genuinely unsupported**: line-level move/copy tracking (`git blame -M`/`-C` — lines that
  moved within a file, or were copied from another file). libgit2's `GIT_BLAME_TRACK_COPIES_*` blame
  option flags exist in the header but are explicitly documented upstream as "not yet implemented."
  This is the part that would actually require an exec (`git blame --line-porcelain`) fallback, and
  it's still not implemented — this decision only closes the whole-file-rename gap.
- **`BlameRange` gained `orig_path: Option<String>`** (`api/src/repo/blame.rs`) — the path a hunk's
  commit had at that point in history, from git2's `BlameHunk::path()`, collapsed to `None` when it
  equals the blamed path (the common case: no rename since) or isn't valid UTF-8. Kept per-hunk
  rather than folded into the per-commit cache (`summary`/`author`/`authored_at`), since the same
  commit can appear under different `orig_path`s in different files.
  `docs/API.md`/`docs/openapi.json`/`web/src/lib/api/types.ts` updated together (API contract
  change).
- **Web**: `BlameView.tsx`'s gutter cell gets a small marker next to the short sha when
  `orig_path` is set, linking to `blameHref(repo, orig_path, sha)` (blame of the old path as of
  that commit) — reusing the existing href helper (`web/src/lib/repo-href.ts`), no new routing.

## #37 Commit log cursor became an offset token (closes the #33 side-branch-drop candidate)

Closes the `docs/ROADMAP.md` candidate noted while building the graph column (#33): commit log
pagination silently dropped a side-branch commit pending at a page boundary that wasn't an
ancestor of the next page's cursor.

- **Root cause, confirmed against libgit2 1.9.6's `revwalk.c` directly**: `commits.rs::log` walks
  with `GIT_SORT_NONE` (deliberately, #33 — any sort flag forces a full-history walk before the
  first commit is emitted). That still isn't an unordered walk: `revwalk_next_unsorted` pops from a
  pending list libgit2 maintains in commit-date order (`git_commit_list_insert_by_date`), i.e. a
  date-priority queue. Once `limit` commits are emitted, the old cursor kept only the single
  boundary commit's sha; the next page's `revwalk.push()`'d only that sha, silently losing every
  *other* commit still in the pending list — a sibling that isn't its ancestor never gets pushed
  again. Minimal repro: root `A`, `B(A)`, `S1(A)`, `S2(S1)`, `C(B)`, merge `M(C, S2)`, `limit=2`.
  Page 1 emits `M, C`; the pending list is now `[S2, B]`; `next_cursor` was `S2` alone, so page 2
  pushes only `S2` and `B` never appears on any page.
- **Fix: the cursor became `"<start-sha>.<offset>"`** (`repo/commits.rs::Cursor`) — every page
  re-walks from the same fixed start commit and skips `offset` filtered commits before collecting
  `limit` more. This is provably lossless: it's a plain continuation of one walk, cut at different
  points, so it can't diverge from an unpaginated walk of the same history. cgit's own `ofs=`
  query param does the same thing.
  - `Cursor::MAX_OFFSET = 100_000` bounds the walk a manipulated cursor can force — same rationale
    as search/stats' scan budgets (#26/#28). Exceeding it is `400 invalid_param`, same as a
    malformed cursor; this also means a client can't page past 100,000 filtered commits, judged an
    acceptable trade given none of search/stats/log allow unbounded scans either.
  - **The old bare-sha cursor format is rejected outright, not accepted as a legacy alias.** The
    cursor is documented as opaque in `docs/API.md`, has shipped for one release cycle, and a
    fallback path would have kept the exact bug this decision fixes reachable for anyone still
    holding an old link.
  - Cost changed from O(1) per page to O(page index × `limit`) — a real regression, but strictly
    better than the O(repo size) a sort flag would force (which #33 already rejected), and
    self-limiting: a client that pages `N` times deep pays for `N` re-walks, not the server eating
    that cost unprompted. Each page is still absorbed by the response cache.
  - **Rejected alternative: a frontier cursor** (encode the whole pending-list sha set at the page
    boundary). Keeps O(page) per page, but two problems killed it: libgit2's `seen` flag — which
    prevents a commit from being queued twice — doesn't persist across requests, so a commit whose
    timestamp straddles a page boundary under clock skew could be emitted twice; and the frontier
    itself needs a size cap, which reintroduces exactly this decision's drop bug once a page
    touches enough diverging branches to exceed it.
  - `commits::log` gained a `skip: usize` parameter; `feed.rs`'s call (fixed `FEED_ENTRY_LIMIT`,
    never paginated) passes `0`.
  - `docs/API.md`/`docs/openapi.json`/`web/src/lib/api/types.ts` updated (description-only:
    `cursor`/`next_cursor` semantics changed, `CommitsPage`'s shape did not).
  - **No web change**: `CommitLog.tsx`'s `Older` link and `logHref` already treat `next_cursor` as
    an opaque string round-tripped through `?cursor=`, never parsed client-side.

## #38 raw patch and rawdiff: in-process git2, not exec; email exception on `/patch`

Part of the "diff and patch output" cgit-parity work (ROADMAP.md), alongside `GET /diff` (#2-rev
diff) and `context=`/`ignorews=` (#1). Two new endpoints, `GET /rawdiff` and `GET /patch`, needed
three decisions that cut against or sit next to existing invariants.

- **`git2::Diff::print`/`Email::from_diff` in-process, not `git format-patch`/`git diff` exec.**
  The #2/#12/#13 hybrid policy reserves exec for heavy streaming operations libgit2 has no
  equivalent for (archive, upload-pack); it doesn't apply here; libgit2 has both operations
  natively. Exec'ing git would also introduce a **second diff engine** disagreeing with the first:
  git's rename detection and libgit2's `find_similar(None)` aren't guaranteed to agree, and git's
  `diff.indentHeuristic` has defaulted on since 2.14 while libgit2's `GIT_DIFF_INDENT_HEURISTIC`
  defaults off. Two engines would mean the structured JSON diff and its `.patch` link on the same
  page could show different file lists or hunk boundaries — a correctness bug, not a style choice.
  Staying in-process also means `/rawdiff` and `/patch` inherit `cached_response`'s response cache,
  `ETag`, and immutable `Cache-Control` for free, rather than reimplementing archive's
  resolve → ETag-short-circuit → spawn → reaper → stream path for an operation that doesn't need
  streaming. `repo/diff.rs::build_diff` is the single git2 diff chokepoint every entry point
  (commit diff, two-revision diff, rawdiff, and each commit in a patch series) now shares.
- **No line/file caps on `/rawdiff` or `/patch`**, unlike the structured JSON diffs' `MAX_DIFF_FILES`
  (300) / `MAX_FILE_DIFF_LINES` (1000). Those caps exist to bound a browser-rendered payload; a
  patch with hunks silently dropped is a *corrupt* patch that fails or misapplies under `git
  apply`/`git am`, defeating the endpoint's reason to exist. `/patch`'s commit-*count* axis is
  bounded instead (`MAX_PATCH_COMMITS = 100`) — but by **rejecting** an oversized range with `400
  invalid_param` rather than truncating it, since a silently-shortened series would still apply
  cleanly and just quietly corrupt history. `raw` already carries the same unbounded-body exposure
  with no reported problems; if `/rawdiff` ever needs one, the natural next step is the same
  exec+stream escape hatch ROADMAP already reserves for search/stats/log, not a cap.
- **`from` means the opposite thing on `/rawdiff` vs. `/patch`.** On `/diff`/`/rawdiff`, `from` is
  the other side of a two-dot tree comparison (`git diff <from> <to>`), matching `GET /diff` (#2).
  On `/patch`, `from` is the **excluded** start of a commit range (`git format-patch <from>..<to>`).
  This matches git's own two conventions exactly, but it is the single most confusable part of the
  API surface — spelled out with a worked example in `docs/API.md` rather than left implicit.
  `/patch` also diffs every commit in its range against its own first parent, including merges;
  `git format-patch` itself skips merges, but every other diff in this API is first-parent, so
  `/patch` stays consistent with axgit's own convention instead of git's.
- **`/patch` is a deliberate, narrow exception to "email addresses are never exposed"** (#8).
  `git am` cannot preserve authorship without a real `From: Name <email>` header — that's the
  endpoint's entire reason to exist, so redacting the address (e.g. `<redacted@invalid>`) would
  produce a patch that applies but records the wrong author, which defeats the point as
  thoroughly as capping the size would. The exception is narrow: nothing new is actually
  disclosed, since the identical data is already served, unauthenticated, by `git clone` over
  Smart HTTP (#13) — every commit's plaintext author email is inside every packfile. What changes
  is *harvesting economics*: a plaintext address sitting behind a plain `GET` is crawler-trivial in
  a way a git pack is not. Mitigated with `X-Robots-Tag: noindex, nofollow` on the response (one
  header) and `Content-Disposition: inline; filename="..."`; the pre-existing `robots.txt` gap
  (ROADMAP.md parity notes) remains the more complete fix and is left as a candidate. Every other
  response (`/repos`, `/commits`, `/diff`, etc.) keeps `email_hash` only — this exception is scoped
  to `/patch` alone.
- **`sanitize_component`** moved from `handlers/archive.rs` to `handlers/mod.rs` (unit tests moved
  with it) so `/patch`'s `Content-Disposition` filename can reuse the exact ASCII-safe
  slash-replacing logic archive downloads already use, instead of a second implementation.
- API contract change — `docs/API.md`, `docs/openapi.json`, and `web/src/lib/api/types.ts` all
  updated together; two new operations (`EXPECTED_OPERATIONS` in `api/tests/openapi_test.rs`).

## #39 Compare page, side-by-side view, and Shiki highlighting in diffs

The web half of the "diff and patch output" cgit-parity work (#38), landed as four more staged
commits: shared diff-rendering extraction, display options + patch/rawdiff links on the commit
page, the `/{repo}/diff` compare page, side-by-side view, and finally syntax highlighting. No API
contract change in any of these — all four consume endpoints #1/#2/#38 already expose.

- **`/{repo}/diff` is a real route, not a query param on an existing page** — a `Diff` tab plus
  `shellFor`/`shell_for` (the three-places-at-once rule, CLAUDE.md). The alternative (folding
  compare into the commit page behind a query flag) was rejected: a comparison between two
  arbitrary revisions isn't "a commit," and reusing the commit shell would mean every commit-page
  assumption (a single `sha`, a `parent`) has to become conditional. cgit's own two-revision shape
  (`cmd=diff&id=&id2=`) is remapped here instead of onto the single-commit view — `id`/`id2` never
  collide with the compare page's own `from`/`to` query keys, checked directly in
  `cgit_compat.rs`/`cgit-compat.ts` rather than assumed safe.
- **The `(diff)` link on a commit's parent row needs its own accessible name.** Once a page can
  have both a `Diff` tab (name "Diff") and one `(diff)` link per parent, a same-or-overlapping
  accessible name for all of them is an accessibility-tree ambiguity as well as a guaranteed
  Playwright strict-mode failure. Each `(diff)` link's name is `Diff against parent <sha>`
  (`aria-label`, not the visible text) — the same treatment given to the commit page's new `Tree`
  link, which otherwise collides with the `Tree` tab's own name.
- **Side-by-side pairing reuses cgit's `ui-ssdiff.c` algorithm**: consecutive deletions and
  additions are collected separately and paired index-for-index once the run ends, rather than
  assuming a hunk alternates `-`/`+` one-for-one — a 3-deletion/1-addition block becomes one paired
  row plus two `{ new: null }` rows instead of silently dropping two deleted lines.
- **Intra-line highlighting is hand-rolled (`lib/diff/intraline.ts`), not a dependency.** Three
  tiers — common prefix/suffix trim, then a budget-capped (250,000 char-product) word-level LCS,
  then a whole-middle-changed fallback past that budget — cover the common cases at a fraction of
  the size of `diff`/`diff-match-patch`. Consistent with #19's choice of Shiki's JS-regex engine
  specifically to avoid a large payload for a diff-adjacent feature; shipping a diff library here
  would undercut that same rationale.
- **Shiki highlighting reconstructs each file's shown lines per side, not per line.** `context +
  deletion` and `context + addition` are each joined into one string and tokenized once
  (`lib/diff/file-highlights.ts`), then mapped back to their originating `Line` object by identity
  — a multi-line construct (an unterminated string, a block comment) is far less likely to be
  mis-highlighted with surrounding context than tokenizing every line in isolation. Only the lines
  actually present in the diff are included, so a hunk boundary can still land mid-construct; that
  residual inaccuracy is accepted rather than fetching and tokenizing the full blob for both sides
  of every file in a diff.
- **Shiki's foreground color and the intra-line background are composed, not nested.** Both are
  independent segmentations of the same line; `lib/diff/merge-tokens.ts` slices each at the other's
  boundaries so a single pass of `<span>`s carries both `style` (Shiki) and a `changed` flag
  (intra-line), rather than trying to render one segmentation's spans nested inside the other's.
- **Added/deleted files always render unified, regardless of the page's chosen view.** A wholly new
  or removed file has nothing on one side either way — an entire empty split column is pure waste,
  and cgit doesn't have a "split" concept for these either.
- Verified against a real end-to-end run (fixture repo, built `web/dist`, `cargo run`, a real
  browser) in addition to the mocked component/e2e tests — the split view's intra-line spans and
  Shiki's per-token color were checked to actually compose correctly on screen, not just assumed
  from the two algorithms' unit tests in isolation.

## #40 "Compare" entry point on the refs page: two independent directions, `getRepo` as decoration

Closed one of the two candidates #39 deferred (the other, prefilling the idle compare page's `to`
from the default branch, is unchanged). Web-only — `RefsView.tsx` is the only file with real
logic changes.

- **`getRepo(repo)` is fetched in its own effect, parallel to and independent of `RefsView`'s
  existing `getRefs` fetch**, purely to read `default_branch` — `RefsInfo` has no such field. This
  is decoration, not required data: a failure never touches the page's loading/error state, it just
  leaves the new Compare column's cells empty. Same rule already applied to `useCommitRefs` (#34)
  and to `DiffView`'s own parallel `getRefs` call for its revision datalist — not a new pattern,
  a third application of one.
- **Branches and tags compare in opposite directions.** Branches: `from={default_branch}`,
  `to={branch.name}` — "what does this branch have that the default doesn't." Tags:
  `from={tag.name}`, `to={default_branch}` — "what's landed since this tag." A tag almost always
  points at an ancestor of the default branch, so using the branches' direction for tags too would
  make most tag comparisons show an empty or backwards-looking diff, forcing a `Swap` click every
  time. Both reuse the existing `compareHref` (`lib/repo-href.ts`) unchanged.
- The default branch's own branch row gets no Compare link (self-comparison is always empty) — an
  em dash, matching the tables' existing missing-value convention, rather than a link to a diff
  that's guaranteed to render nothing.
- **Each link's accessible name is `Compare {from} with {to}`, via `aria-label`** — identical
  reasoning to #39's commit-parent `(diff)` links: a table full of identically-named "Compare"
  links is an accessibility-tree ambiguity as well as a guaranteed Playwright strict-mode failure.
- No new route, so `shellFor`/`shell_for`/`RepoNav.astro` are untouched; no API contract change.

## #41 Idle compare page prefills `to` from the default branch; `useDefaultBranch` extracted

Closed the other candidate #39 deferred (#40 closed the first). Web-only, no API contract change.

- **Prefills `to`, not `from`** — mirrors the api's own default direction (`docs/API.md`: `to`
  defaults to `HEAD`, `from` defaults to `to`'s first parent). This makes that default visible
  rather than inventing a new comparison direction of its own.
- **No auto-fetch.** `/{repo}/diff` with no query params still means "idle" — only the `to`
  input's value and the idle copy change; `state` stays `"idle"` until a `Compare` submit.
  `useDefaultBranch` is only enabled while idle, so a page that already has a comparison makes no
  extra request.
- **`web/src/lib/default-branch.ts::useDefaultBranch`** — extracted out of `RefsView.tsx`'s
  `getRepo` effect (added in #40) once `DiffView` became a second caller, following
  `useCommitRefs`'s (#34) precedent for "decoration fetch as a shared hook": `null` while loading,
  on failure, for an empty repository, or when disabled; failures are silent.
- **The prefill is imperative (a ref), not `defaultValue`/`key`.** The form is deliberately
  uncontrolled (plain `method="get"`, works without JS, #39), and React ignores a changed
  `defaultValue` on re-render, so the async default branch would never reach the input that way. A
  `key` remount would show it, but would also discard anything the visitor already typed while the
  fetch was in flight, and steal focus. Verified safe against the installed `@base-ui/react`
  `Input` source: uncontrolled, it renders a native `<input defaultValue>` with no React state for
  the value (only `dirty`/`filled` data-attributes, which require a `Field.Root` this form doesn't
  use) and forwards the ref straight onto the input element. The effect only assigns when the
  field's current value is still `""`, so it never clobbers manual input, and the existing e2e
  `fill()`-then-submit test stays race-free.
- No new route, so `shellFor`/`shell_for`/`RepoNav.astro` are untouched.

## #42 Commit log pages are immutably cached when the request pins the walk start (closes #37's follow-up)

Closes the `docs/ROADMAP.md` candidate deferred alongside the moka cache rollout (#6) and again
when #37 landed: cursor- and full-sha-`ref`-addressed commit log pages are now immutable, matching
commit detail/diff/blame/search/stats.

- **The premise for deferring this was stale.** The original justification was that promoting these
  pages "would change the API.md contract (sha appears in the URL path)" — but `docs/API.md`
  already documents query-param-driven immutability for `GET /diff?from=&to=` (#38), `/search?ref=`
  (#26), and `/stats?ref=` (#28); the generic "Caching headers" bullet just hadn't been reworded to
  match. It now reads "pins the resource to a full sha — whether a path segment or a query
  parameter" instead of "the requested path value".
- **Rule**: `cursor.is_some() || ref == Some(resolved_start_sha)`, computed in
  `handlers/commits.rs::list_commits` right after `commits::log` returns. `cursor` is unconditional
  because the token already encodes a full-sha walk start (`Cursor::parse`, #37) — `ref` is ignored
  whenever a cursor is present, so nothing else can make the page depend on it. A bare `ref` full-sha
  match mirrors `search.rs`/`stats.rs` exactly. The empty-repository page (no `ref`, no `cursor`)
  stays mutable — it has no pinned start at all.
- **Why cursor-always is sound.** `commits::log` is a pure function of the object graph reachable
  from `start` (#37 already established this walking libgit2 1.9.6's `revwalk_next_unsorted`
  directly: the pending list's order depends only on commit objects, not refs/HEAD/packfile
  layout); `touches_path` only compares tree-entry oids between a commit and its parents. So for a
  fixed `start`, the page is `f(start, offset, path, limit)` — `path`/`limit` are already in the
  cache key and don't need to gate immutability. A cursor pointing at a GC'd or dangling commit
  behaves exactly like `/commits/{sha}` today (`cache_test.rs`'s
  `sha_addressed_diff_should_hit_without_touching_repo` already relies on this): a fresh compute
  would 404/400, but an already-cached immutable entry keeps serving until TTL/eviction.
- **Blast radius is bounded server-side.** `cache.rs::build_response_cache` puts `.time_to_live(ttl)`
  on the whole moka cache, and immutable entries go through the same `insert` as everything else —
  so a wrong or stale immutable body only survives one `AXGIT_CACHE_RESPONSE_TTL` window (300s
  default) server-side, plus the byte-weigher/32 MiB cap. Only the *browser's* copy is genuinely
  pinned for a year. The one real behavior change: today a push evicts every cursor entry via the
  validator; afterwards, a cursor page's own body can't change from a push, so it survives until
  TTL/LRU instead — page 1 (no `ref`/`cursor`) still invalidates on HEAD move and produces a
  different `next_cursor`, so an active repo's paging chain still self-refreshes from the tip.
- **No web change.** `CommitLog.tsx` already round-trips `next_cursor` as an opaque string via
  `logHref`; the web essentially never sends a full-sha `?ref=` (ref selection is name-based, #18;
  the only realistic path is a hand-typed URL or the cgit-compat `h=<sha>` redirect, #35), so the
  practical win is the cursor half — the "Older →" chain's deep pages (walk cost is O(page × limit),
  #37) stay warm across pushes and skip the repo-open/validator round trip entirely on a hit.
- `docs/API.md`/`docs/openapi.json`/`web/src/lib/api/types.ts` updated (description-only: the
  `/commits` 200 response's `ETag`/`Cache-Control` header descriptions, plus a new caching bullet in
  the endpoint's own section — `CommitsPage`'s shape did not change).

## #43 Diff stat-only mode (`view=stat`), with a `stat=1` fast path on `GET /diff`

Closed the last `docs/ROADMAP.md` candidate on the diff/patch surface — cgit's `dt=2`. Two-part
change, api then web, following #38/#39's precedent.

- **The api half wasn't optional.** The candidate note reasoned that `/commits/{sha}`'s diffstat
  and `/diff`'s `diffstat` field already carry everything a stat view needs, so this could be pure
  client-side rendering — true for the per-commit page, but not for the compare page: stat mode's
  entire reason to exist is "this diff is too big to render," which is exactly the case where
  `GET /diff` would still ship up to 300 files × 1000 rendered lines of hunks the page throws away.
  So `GET /diff` gained `?stat=1` (`repo/diff.rs::rev_diff_stat`, a sibling of `rev_diff` sharing a
  new `rev_sides` helper for the (old tree, old sha, new tree) triple) — skips `render_files`
  entirely, always returns `files: []`/`truncated: false`, and computes only the already-uncapped
  `diffstat`. A separate function rather than a boolean flag on `rev_diff`, so "hunks are never
  rendered in this mode" is a property of the call graph, not a runtime branch to get wrong.
  `stat` is part of the cache key (`revdiff_test.rs` asserts the full and stat-only responses for
  the same revisions are independently cached) and factors into the immutable-caching decision the
  same way `context`/`ignorews` always have — no special-casing needed there since `stat` doesn't
  change what `from`/`to` resolve to.
- **`/rawdiff`'s query struct was split off** (`RevDiffQuery` → `RevDiffQuery` for `/diff`,
  `RawDiffQuery` for `/rawdiff`), rather than adding `stat` to the shared struct both handlers used.
  `stat` has no meaning on `/rawdiff` (a plain-text patch has nowhere to put a stat summary, and
  #38 already ruled out ever truncating it) — leaving it on the shared struct would have put a
  meaningless parameter in that endpoint's own OpenAPI spec.
- **The commit page and compare page take opposite approaches to the fetch itself**, and that
  asymmetry is deliberate rather than an inconsistency to fix later: `CommitView.tsx` already has
  `detail.diffstat` (uncapped) from its existing `getCommit` call, so stat mode there skips the
  `getCommitDiff` request outright — no api parameter needed, since there's nothing left to ask
  for. `DiffView.tsx` has no equivalent standalone diffstat call, so it fetches `GET /diff` with
  `stat=1` and, deliberately, without `context`/`ignorews` — neither affects `diffstat`
  (`diffstat_for_trees` always uses `DiffParams::default()`), and dropping them normalizes every
  stat-only comparison for a given `from`/`to`/`path` onto one cache entry regardless of what
  unified/split had last been set to.
- **`view` gained a third value (`"stat"`)** rather than a separate boolean alongside it
  (`web/src/lib/diff-options.ts`) — `DiffOptionsBar`'s pill row, the URL round-trip, and the
  hidden-input carry-through in `diffOptionsQuery` all already treat `view` as the one
  display-mode axis, so a third pill was strictly additive. `DiffFileList`/`DiffFile` narrow their
  own `view` prop to a new `HunkViewMode = Exclude<DiffViewMode, "stat">` type instead of handling
  a meaningless third case internally — stat mode never renders them at all, so the type system
  states that rather than a runtime guard.
- **Each stat row links to that file's own single-file diff** (`path=` + `view=unified`), not to a
  `#diff-N` anchor — there is no file list on the page in stat mode for an anchor to jump to.
  `DiffStatTable` gained an optional `hrefFor` (default: the existing anchor, unchanged for every
  other caller) rather than becoming stat-aware itself. This is also why `commitHref` gained a
  `path` param (`compareHref` already had one) and why `CommitView`/`DiffView` both gained `?path=`
  support entirely — previously nothing on the commit page could restrict its diff to one file.
  Landing on a path-filtered single-file diff (from any view, not just stat) now shows a
  "Showing only `{path}` — Show all files" line; without it, the only way back to the full diff
  would have been the browser's back button.
- Verified end-to-end against a real built `web/dist` served by the api over the fixture repos
  (not just mocked component/e2e tests): confirmed via the browser's own network panel that
  `view=stat` sends `stat=1` with no `context`/`ignorews` on the compare page and issues no
  `/commits/{sha}/diff` request at all on the commit page.
- `docs/API.md`/`docs/openapi.json`/`web/src/lib/api/types.ts` updated (`GET /diff` gained `stat`).

## #44 Log tab message expansion (`msg=1`), cgit's `showmsg=1` parity

Closed the "Log → Expand full commit message" `docs/ROADMAP.md` cgit-parity gap. Git notes stayed
out of scope — cgit's `showmsg=1` renders the body plus a note row, but axgit has no `refs/notes`
reading anywhere in the api, git-compose's stack never produces notes, and the ROADMAP already
tracks notes display as its own gap on the commit page. Closing only the body half keeps this
change to one concern.

- **`CommitInfo` gained an optional `body`, keyed off a new `msg=` query param on `GET /commits`**
  (`api/src/repo/commits.rs`, `api/src/handlers/commits.rs`) — `0`/`1`/`true`/`false` via the
  existing `parse_flag` helper (shared with `ignorews`/`stat`), default off. The default (no `msg`)
  log payload is unchanged, and so is `/search?type=message`'s response, which reuses `CommitInfo`
  through the same `commit_info` constructor — a new `commit_info_with_body` sibling is used only
  by the log walk when `msg=1` is set, so search never has to think about the field at all.
- **`body` is an omitted key, not a `null` value, when there's nothing to show** — off by request,
  no body on the commit, or a non-utf8 message all serialize the same way
  (`#[serde(skip_serializing_if = "Option::is_none")]`, no `#[schema(required = true)]`). This
  breaks from every other optional field in `CommitInfo`/`CommitDetail`, which are always-present
  keys that can be `null` — deliberate here, since a caller that didn't ask for `msg=1` shouldn't
  see a `body: null` on every single entry it has no use for, and a caller that did ask can treat
  "key present" as "there's something to render" without also checking for `null`.
- **`msg` is folded into the cache key's `params` string** (`commits.rs`'s hand-built format string)
  the same way `stat` was for `GET /diff` (#43) — otherwise a `msg=1` response and the default one
  for the same `ref`/`path`/`cursor`/`limit` would alias onto the same cache entry.
  `commits_uses_a_separate_cache_entry_for_msg` (`api/tests/commits_test.rs`) asserts this the same
  way `revdiff_test.rs` does for `stat`. `msg` does **not** affect the immutable-vs-`ETag` decision
  (`docs/DECISIONS.md` #42) — same reasoning as `path`/`limit`: for a fixed walk start it only picks
  which fields come back, not which commits.
- **The graph column needed a second row shape.** `CommitLog.tsx`'s existing `TableRow`/`h-12`/
  `CommitGraph` combination is a fixed-height grid (`docs/DECISIONS.md` #33) that an arbitrary-length
  message row can't fit into, and hiding the graph for expanded rows (the path-filter precedent)
  would have broken the line for every commit, not just the ones with visible bodies. Instead
  `CommitGraph.tsx` gained a sibling `CommitGraphSpacer`: an absolutely positioned
  (`absolute inset-y-0 h-full`) SVG holding only straight verticals for `row.through ∪ row.out` — no
  node, no diagonals, because those belong to the commit row above — stretched to whatever height
  its container's message content needs by CSS rather than by a fixed `viewBox`. Verified in a real
  browser against `devlog-nextjs` (dependabot bodies running to 20+ lines of linkified URLs): the
  line stays a single unbroken column through message rows of any height, in both themes and past
  a cursor page boundary.
- Each commit is a `Fragment` wrapping its own row plus an optional message row
  (`expanded && commit.body`), rather than a second `.map()` pass or a flag on `CommitGraph` itself
  — keeps "does this commit have a body row" a single `&&` next to the row it belongs to.
- **Toggle is a URL-only link** (`Expand messages` / `Collapse messages`, `logHref`'s `msg` param),
  not client component state — same "display option lives in the URL, no state to fall out of sync
  on navigation" rule `DiffOptionsBar` established (#24) and `stat=1` (#43) reused. The link
  preserves `ref`/`path`/`cursor`, so toggling from a paginated ("Older →") page doesn't reset to
  page 1, and the "Older →" link itself carries `msg` forward the same way it already carries
  `cursor`.
- Verified end-to-end against a real built `web/dist` served by the api over the fixture repos: the
  default log has no `body` in the network response, `?msg=1` adds it and renders the expansion,
  `Collapse messages` on a cursor page drops `msg` while keeping `cursor`, and the graph line holds
  together across both plain and very long dependabot-style bodies.
- `docs/API.md`/`docs/openapi.json`/`web/src/lib/api/types.ts` updated (`GET /commits` gained `msg`,
  `CommitInfo` gained optional `body`).

## #45 Git notes on the commit page (cgit's `format_display_notes()` parity)

Closed the last `docs/ROADMAP.md` "Commit page" cgit-parity gap, deferred by #44. Scope is the
commit page only — the log's `msg=1` rows stay note-free, kept as its own possible future item.

- **A note is mutable state on an otherwise immutable resource.** `GET /commits/{sha}` has been
  served immutably (`max-age=31536000`, no `ETag`) whenever `{sha}` is the resolved full sha, on
  the premise that a fixed commit object is a pure function of its sha (#42's same reasoning). A
  `git notes` message is not part of the commit object — it lives on a separate ref
  (`refs/notes/commits`) and can be added, edited, or removed without the commit sha changing, so
  that premise breaks the moment a note exists. `handlers/commits.rs::get_commit`'s immutability
  flag is now `sha == detail.sha && detail.note.is_none()`: a note-less commit (every repository
  git-compose produces today) behaves exactly as before; a noted commit falls back to `ETag` +
  `no-cache` unconditionally, so it always revalidates against the repo's HEAD+agefile validator —
  which does change on a notes push, since the post-receive hook touches the agefile on every push
  to the repo, not just to `main`.
  - Considered and rejected: a dedicated `GET /commits/{sha}/notes` endpoint (keeps detail's
    immutability pure, mirrors `/diff`/`/patch`/`/rawdiff` sibling endpoints) — rejected because it
    would cost the commit page a second parallel request for what's usually nothing, and the
    `msg=1`/`stat=1` precedent (#43/#44) is opt-in query params, not sibling routes, for exactly
    this "small, commonly-absent extra field" shape. Also considered a `notes=1` opt-in param
    matching that precedent — rejected because the web always wants the note when present (there's
    no stat-view-style reason to omit it), so an opt-in param would just make the commit page
    request it unconditionally, adding a query string with no actual behavior choice behind it.
  - Accepted, documented limitation: a browser that cached a commit page **before** a note was
    added keeps serving the note-less copy for up to a year — inherent to committing to immutable
    caching for full-sha resources at all (#42), not new here. Server-side blast radius stays
    bounded the same way #42 reasoned for a stale immutable body: the moka entry survives one
    `AXGIT_CACHE_RESPONSE_TTL` window (300s default) before a fresh compute picks the note up.
- **Only the default notes ref is read** — `Repository::find_note(None, oid)`, which resolves
  libgit2's default (`core.notesRef`, else `refs/notes/commits`). cgit additionally honors
  `notes.displayRef`/`GIT_NOTES_DISPLAY_REF` and concatenates multiple refs; axgit reads one,
  matching the project's general stance of not reproducing cgit's full config surface
  (`docs/ROADMAP.md`'s "Replaced / no analogue planned" list already excludes most of cgit's
  cgitrc-only knobs). Any lookup failure — no notes ref, no note on this commit, non-UTF-8 message,
  an all-whitespace note — collapses to `None`, never an error: a repository with no notes ref at
  all must not turn a working commit page into a 500.
- **`note` is a required-but-nullable field**, not an omitted key — unlike `CommitInfo::body`
  (#44), which is opt-in via `msg=1` and only present when there's something to show. `note` is
  never opt-in: every `GET /commits/{sha}` response carries the key, `null` when there's nothing to
  render, matching every other optional field on `CommitDetail` (`message`, `authored_at`, ...).
- **Rendered as its own block**, not folded into the message `<pre>` — a left accent border
  (`border-l-4 border-l-primary`) under a "Notes" heading, directly below the commit message and
  above the diffstat, linkified through the same `linkify()` the message already uses (cgit's
  `format_display_notes()` also runs plain-text `html_txt`, no markdown). Keeping it visually and
  structurally distinct from the message matters: a note is not something the author wrote when
  making the commit, cgit's own `notes-header`/`notes` CSS classes make the same separation, and
  the message `<pre>`'s `{detail.message && ...}` render is untouched — a note doesn't retroactively
  make a message block appear.
- Fixtures: `scripts/make-fixtures.sh` attaches one note (`Reviewed-by: PtCookie`) to
  `git-compose.git`'s `feat: add compose file` commit and pushes `refs/notes/commits` alongside
  `main`/the tag — notes live on their own ref, so no existing fixture commit sha changed.
- `docs/API.md`/`docs/openapi.json`/`web/src/lib/api/types.ts` updated (`CommitDetail` gained
  `note`; the endpoint's caching description now states the extra "note-less" condition).

## #46 Search API: `type=author|committer|range` (cgit `qt=author|committer|range` parity)

Closed the `docs/ROADMAP.md` "Log" cgit-parity gap: `/search` (#26) only covered `content`/`path`/
`message` (cgit's `grep`), leaving cgit's two remaining log query types — search by author/
committer, and a rev-list range expression — with no analogue. Same endpoint, same response
envelope (`sha`, `truncated`, `files`, `commits`), following #26's own "one endpoint, one type
selector" rationale rather than three more routes.

- **`author`/`committer` match the signature *name* only, never the email.** axgit's blanket
  invariant is that no response ever exposes a raw email address (only `email_hash` —
  `repo/commits.rs`'s `CommitAuthor`); matching the email in a search box would turn it into a
  confirm/deny oracle for a specific address (search `alice@example.com`, see whether any commit
  comes back), which the hash-only design exists specifically to prevent. This is a deliberate
  difference from cgit's own `--author=`, which matches `Name <email>` — documented in `docs/API.md`
  so it isn't mistaken for a bug later. Implementation is the same revwalk-and-budget shape
  `search_messages` already used (`repo/search.rs::search_signatures`), just testing
  `commit.author()`/`committer()`'s `name()` instead of `message()`.
- **`range` treats `q` as a rev-list expression, not a text filter** — the query *selects* commits
  directly (`A..B`, `A...B`, `^X`, bare revs, space-separated), matching cgit's own `range` mode.
  `ref` still resolves the response's `sha` (so the empty-repository carve-out and the general
  "`ref` defaults to HEAD" behavior are untouched) but plays no part in the walk itself — `q` alone
  has to name everything to include, same as `git log <range>` on the command line ignores any
  checked-out branch.
  - **A token starting with `-` is `400 invalid_param`.** Nothing here is ever exec'd (`repo/
    search.rs`'s doc comment already states the container never shells out to `git log` for
    search), so this isn't an injection guard — it exists purely so a `git log`-flag-shaped token
    (`--all`, `-n5`) gets a clear 400 instead of libgit2 attempting to `revparse` it as a revision
    and failing with a confusing `ref_not_found`.
  - **Hand-rolled `A..B`/`A...B`/`^X` parsing (`repo/search.rs::walk_range`), not
    `git2::Revwalk::push_range`.** Reading libgit2 1.9.6's `revwalk.c` directly (the same
    verification standard #24 established) showed `git_revwalk_push_range` explicitly rejects
    `A...B` — `GIT_REVSPEC_MERGE_BASE` hits a `goto out` with "symmetric differences not
    implemented in revwalk" (`revwalk.c:253`) — so it can't cover the full grammar the plan called
    for. Rather than mixing `push_range` for `..` with hand-rolled logic only for `...`, both are
    parsed by hand for one uniform code path, one error type
    (`resolve::resolve_commit`'s existing `RefNotFound` mapping — no bare `?` anywhere, so an
    unresolvable token in a range is `404 ref_not_found`, never a 500), and one consistent
    "empty side means `HEAD`" rule across `..`/`...`. `A...B`'s symmetric difference is built from
    `Repository::merge_bases` (plural — a criss-cross history can have more than one base) with
    every base `hide()`-ed before both sides are `push()`-ed.
  - Every revision goes through the same `resolve::resolve_commit` every other endpoint uses, so
    branch/tag/short-sha/`HEAD` all resolve identically to `?ref=` elsewhere in the API — no
    parallel resolution logic.
- **`range` never gets immutable caching, even with a full-sha `ref`.** Immutability up to now has
  meant "the request's `ref` is the resolved commit's own sha," which was a sound proxy because the
  API's own resolution of `ref` was the only thing feeding the response. `range` breaks that proxy:
  the actual result depends on whatever revisions `q` names (`main~5..main` moves every time `main`
  moves), which is completely independent of `ref`. `handlers/search.rs::get_search` special-cases
  it: `immutable = kind != SearchKind::Range && ref == sha`. The existing cache key already
  includes `q`/`type`, so this only affects the immutable-vs-ETag header choice, not correctness of
  what's cached.
- **`MAX_SCANNED_COMMITS` (10,000) and `limit` apply to `author`/`committer`/`range` exactly as
  `message` already used them** — no new budget constant, same `truncated` semantics.
- `docs/API.md`/`docs/openapi.json`/`web/src/lib/api/types.ts` updated (new `SearchKind` variants,
  the `type`/`q`/caching prose extended). The `/{repo}/search` web page picking up the three new
  types is a follow-up commit (#47).

## #47 `/{repo}/search` page: surface `type=author|committer|range`

The web half of #46 — the API commit deliberately deferred the UI, following #26/#27's own
API-then-page precedent.

- **No new component.** `type=message`'s commit-row renderer in `SearchResultsList`
  (`SearchView.tsx`) already fits `author`/`committer`/`range` exactly — all four produce
  `SearchResults.commits` in the identical `CommitInfo` shape. The `results.type === "message"`
  branch condition became a `COMMIT_KINDS` membership test (`["message", "author", "committer",
  "range"]`) instead of gaining three more `||` arms or a second renderer.
- **`TYPE_OPTIONS` stayed the single source of truth for valid `type` values.** `isSearchKind`
  previously hard-coded its own three-way `===` chain, which would silently drift from
  `TYPE_OPTIONS` the moment one list was updated and the other wasn't (exactly what this commit
  would have done to it). Replaced with `SEARCH_KINDS = TYPE_OPTIONS.map(o => o.value)` and an
  `.includes` check, so adding a `type` only ever means editing `TYPE_OPTIONS`.
- **A one-line hint under the form for `type=range`.** Every other type's query is a plain
  case-insensitive text match, which the search box's placeholder already conveys; a rev-list
  expression (`v1.0..main`, `main ^next`) is a different enough input shape that the box alone
  gives no clue what's expected. Shown whenever `resolvedType === "range"` (the URL-derived state
  the uncontrolled form already reads for its `defaultValue`), not tied to live `<select>` input —
  consistent with the rest of the page having no controlled/live form state (#27).
- No route/`shellFor`/`shell_for` change (the shape is unchanged: `?q=&type=&ref=`), no API
  contract change — `web/src/components/repo/SearchView.tsx` and its test only.

## #48 Per-row quick links on the repository index and the tree listing

Closed two independent-but-similar cgit parity gaps at once: `enable-index-links` (repository index)
and the tree listing's missing log/raw/blame links (axgit only had those on the blob page).

- **Shared `IconLink` component, not a `RowActions` component.** Both tables needed the same
  accessibility contract — an icon-only link whose name comes entirely from `aria-label`, the icon
  itself `aria-hidden` — so that got pulled into `web/src/components/IconLink.tsx` (top-level, not
  `components/repo/`, since the repository index isn't a repo page). What was **not** shared is which
  actions apply to which row: the tree listing's set varies by entry kind (submodule rows get
  nothing, directories get Log only, blobs/symlinks get Log + Raw + Blame) while the index has no
  kind at all (every row gets Log + Tree). A component that owned "the action set for a row" would
  have had to parameterize around a concept only one of its two callers has.
- **Directory rows get a Log link too.** `?path=` on `/log` already matches a directory prefix, not
  just a single file, so "this directory's history" is a real, working link — cgit's own tree view
  gives directories a log link for the same reason.
- **No API contract change.** `logHref`/`treeHref`/`blameHref` (`web/src/lib/repo-href.ts`) and
  `rawUrl` (`web/src/lib/api/repos.ts`) already covered every shape needed; `TreeEntryInfo`'s existing
  `name`/`type` fields were enough to build each row's hrefs.
- **Folded in a pre-existing duplication first, as its own commit.** `BlobView.tsx` and
  `BlameView.tsx` each hand-built their "History" link's URL with `encodeSegment` instead of calling
  `logHref`, with a local `const logHref` shadowing the importable name. Fixed ahead of the two
  feature commits — proven byte-identical by the untouched existing tests (`?path=…&ref=…` in that
  exact order, matching `logHref`'s own key-iteration order for that argument shape).
- **New `aria-label`s made several existing non-exact `getByRole("link", { name })` lookups
  ambiguous** (in `RepoList.tsx`'s and `TreeView.tsx`'s tests and e2e specs) — once a row grew action
  links, a bare name like `"main.rs"` became a substring of its own `"Blame for main.rs"` label.
  Switched the affected lookups to `exact: true`, following the precedent `RepoList.test.tsx` had
  already set for the `?q=` filter tests.
- **Skeletons needed no change.** `RepoListSkeleton`/`TreeViewSkeleton` model row *height* as plain
  `h-8` bars, not a mirrored table; a `w-px` icon column adds no height, so DECISIONS #17/#31's
  no-reflow rule was already satisfied.

## #49 Symlink targets in the tree listing (cgit `name -> target` parity)

The blob endpoint has exposed a symlink's target since the file-browsing work — `BlobInfo.content`
*is* the link target, and `BlobView.tsx` renders "Symlink to `<code>`". The tree listing computed the
same information and threw it away, so a client had to open every symlink to learn where it points.

- **Served verbatim, resolved client-side.** `TreeEntryInfo.target` is the raw stored path, relative
  to the entry's own directory, with no server-side normalization — a `../` prefix reaches the
  caller intact. Resolving it server-side would mean deciding what an escaping target (`../../etc`)
  becomes in a JSON field whose whole contract is "this is what git stores", and would lose the
  distinction between a link written relatively and one written from the root.
- **The existing `odb.read_header` call became the size gate.** It was already in the entry loop for
  a blob's `size` and is a stat, not a load; widening it from `Blob` to `Blob | Symlink` bounds the
  target read at `SYMLINK_TARGET_LIMIT` (4096, PATH_MAX) for free. **Over the cap reports `null`
  rather than truncating** — half a path is a *wrong* target, not a shorter one, and a caller that
  linked it would silently point somewhere real but incorrect. Non-UTF-8 collapses to `null` too,
  matching `blob.rs::classify`'s rule for blob content: one nullable field, three benign causes.
- **`size` stays blob-only.** Populating it for a symlink would contradict `docs/API.md`'s "blob
  only" line for a value that is just `target.len()`, and would put "4 B" in the Size column where
  it reads as noise.
- **`resolveRepoPath` was reused, not reimplemented.** `web/src/lib/markdown-url.ts` already resolves
  a repository-relative reference against a base directory — drops `.`, pops on `..`, returns `null`
  on root escape — for README links (#21). Its own docstring anticipated generalizing past the
  empty base; `TreeView.tsx`'s `SymlinkTarget` is that first caller. The base is the **listed
  directory**, not the entry's own path, which is what "relative to the entry's own directory"
  means once the entry is a file inside it.
- **The raw target is displayed, the normalized one is linked** (cgit does the same), and a `null`
  resolution renders as plain text — there is nothing in the tree to point at. A target naming a
  *directory* still gets a blob href: the kind isn't knowable from a listing that only saw the link
  itself, and the blob endpoint 404s cleanly rather than guessing.
- **The test fixture's target was chosen not to collide with any row name.** The target renders as
  its own link, so reusing `README.md` (the actual fixture symlink's target) would re-create #48's
  strict-mode ambiguity across every non-exact `getByRole("link", { name })` lookup in the file.
  `scripts/make-fixtures.sh` gained `docs/readme-link -> ../README.md` so the relative case is
  reachable in a real repository, following #45's git-note precedent.

## #50 `Others (N)` on the stats page, instead of silently truncating authors

`limit` capped the `authors` array and threw the rest away, so the rows on screen never added up to
the Total footer — the bucket totals had always included the cut authors (deliberately), leaving a
visible, unexplained discrepancy on any repository with more than `limit` contributors.

- **A separate `others` object, not a synthetic entry appended to `authors`.** `AuthorStats`
  requires a `CommitAuthor` with a `name` and an `email_hash`; an aggregate has neither. A sentinel
  author would break `AuthorAvatar`, whose identicon seeds from `email_hash`, and would push
  `authors.length` to `limit + 1` — which `StatsView`'s "Showing top N of M authors" hint compares
  against `author_count`. Keeping `others` separate meant that comparison, and the chart (which
  reads only the top-level `buckets`), needed no change whatsoever.
- **`count` is carried explicitly even though it equals `author_count - authors.len()`.** The
  arithmetic is only correct if the caller already knows `authors` is exactly the capped list; an
  object that states its own size is usable without that assumption, and the row's label is
  literally that number.
- **The aggregation is free.** Each `AuthorStats` already carries its own `BUCKET_COUNT`-length
  bucket vector, so `split_off(limit)` hands back a tail that only needs an elementwise sum — no
  second revwalk, no extra accumulator threaded through the walk. `author_count` is read before the
  cut and the top-level `buckets` are built from `bucket_totals`, so nothing else depended on the
  tail still being attached.
- **`others != null` is the precise "`limit` cut authors" signal.** `truncated` deliberately stays
  the union of two causes (scan budget, limit cut) to match search's rule, but callers that need to
  distinguish them now can: `truncated && others == null` is the scan budget alone.
- **The footer still reads the top-level `buckets` rather than summing the rows.** Authors + Others
  now agree with it column for column, but that's a property of the response worth asserting in
  tests, not an invariant to re-derive in the view — a comment above `TableFooter` says so, since
  the reconciliation is exactly the kind of thing a later reader would "simplify".
- The `Others` row renders with no avatar and no name, so it reads as plainly not-an-author.

## #51 Tag detail endpoint + page, and per-tag archive downloads

Closed most of the "Tags and refs" cgit-parity gap: `GET /refs` only ever surfaced a tag's peeled
commit sha and the first line of its message — no tagger, no full message, no way to tell a tag on
a tree/blob apart from one on a commit, and no download link. Three separate commits, landed in
this order: the download links (web-only, no API change), the API endpoint, then the page.

- **`TagRef.target`'s existing meaning (the peeled commit sha) is preserved on the new endpoint
  too, under the same name.** The two designs floated during planning disagreed here — one wanted
  the new endpoint's `target` to mean the tag's one-level dereference. Rejected: `TagRef.target` is
  already documented as "peeled commit sha" (`docs/API.md`) and `web/src/lib/commit-refs.ts` already
  keys ref badges off exactly that meaning. Redefining it on a sibling endpoint would make one word
  mean two things across the same resource family. The one-level dereference instead got its own
  name, `object: { sha, type }` — cgit's own vocabulary (`git cat-file tag` prints `object`/`type`).
- **`target` is `null`, not an error, when the tag never reaches a commit** (a tag on a tree or
  blob). That's deliberately the same condition under which an archive download is unavailable for
  the tag — one field answers both questions.
- **Lightweight tags resolve `200`, not `404`.** `GET /refs` already lists them, so a `404` would
  make every lightweight tag's link on the refs page dead. `tag_object`/`message`/`tagger`/
  `tagged_at` are `null` instead — the precise "this is lightweight" signal.
- **Never immutably cached**, unlike almost every other sha-addressed-looking endpoint in this API.
  The URL names a tag *ref*, not a sha, and a tag can be force-moved onto a different object without
  its name changing — immutability is a property of the address, and this address is mutable by
  construction. (Known pre-existing limitation, not new here: the validator is HEAD sha + agefile
  mtime, so a tag move alone — without a HEAD move or an agefile touch — can stay invisible for one
  `AXGIT_CACHE_RESPONSE_TTL` window, same as `GET /refs` itself.)
- **`repo::tag` is a new module, not folded into `refs.rs`.** `TagDetail` isn't a superset of
  `TagRef` the way `CommitDetail` is of `CommitInfo` — `object` has no analogue on `TagRef` and
  `target` needs the same meaning on both, so there's no natural "extend the log entry" shape here.
  `repo::commits::{signature_info, time_rfc3339}` (already `pub(crate)`) are reused for the tagger,
  keeping the "no raw email address in any response" invariant intact (DECISIONS #8) — `tagger` is
  the same `CommitAuthor` shape as everywhere else.
- **Web: reached only as a drill-down from the Refs page, not a `RepoNav` tab.** `RepoNav.astro`'s
  tabs each carry a single fixed path segment (`Layout.astro` builds `href` as `/${repo}/${sub}`); a
  tag name is unbounded and unknown at build time, so it structurally can't be one. `shellFor`/
  `shell_for` gained a `tag` arm with the same "at least one segment" rule as `blob`/`blame` (a tag
  name may itself contain `/`, e.g. `release/1.0`) — `/{repo}/tag` alone 404s, since the refs page
  already is the tag listing.
- **Only `object.type === "commit"` gets a link on the tag page.** axgit's tree/raw routes are
  ref+path based with no by-oid equivalent, so a tree/blob/tag target is rendered as inert
  monospace text (same shape `TreeView.tsx` already uses for an unlinkable submodule row). This
  narrows, but doesn't close, `docs/ROADMAP.md`'s separate "Object links for non-commit refs" gap —
  left open on purpose rather than growing a by-oid route as a side effect of this page.
- **Download column is tags-only, not also on branches**, matching cgit's own `print_tag_downloads()`
  scope. A branch archive's filename (`{repo}-{branch}.{format}`) names a moving target that changes
  meaning on every push; a tag's is reproducible. `archiveUrl` itself accepts any ref (branches
  included) — this is a scope choice, not a capability gap.
- **Download links use visible text + an overriding `aria-label`** (`Download {tag} as {format}`),
  the table's existing `Compare`-column convention — not `IconLink` (#48). `IconLink` fits when a
  distinct icon carries the row's information; here the format name (`tar.gz`/`zip`) *is* the
  information, and two identical download glyphs side by side would be indistinguishable without a
  hover.
- **Refs page tag names became links** once the API guaranteed every tag resolves (lightweight
  included). This reproduces #48's fallout: a bare `v1.0.0` link is now a substring of both
  `Compare v1.0.0 with main` and `Download v1.0.0 as tar.gz`'s accessible names, so every lookup by
  that name in tests/e2e needed `exact: true`.
- **`RefBadges.tsx` is left unchanged** — its tag badge intentionally links to `logHref` ("commits
  at this ref"), a different intent from the new tag page ("this tag itself"). Noted as a candidate,
  not folded in here.
- Fixtures gained a lightweight, slash-named tag and a tag on a blob (`scripts/make-fixtures.sh`),
  so every response shape — lightweight, slash-in-name, non-commit target — is reachable in a local
  run, not just in tests.

## #52 Remote branches on the refs page, built despite finding no real use case for them

Closed the "remote branches" cgit-parity gap (`enable-remote-branches`) — the last one under "Tags
and refs" besides object links for non-commit refs. Unusual entry: investigated first, found no
evidence the feature is needed, and built it anyway on request as a defensive/future-proofing move
rather than in response to an actual repository that would use it.

- **No repository axgit or the git-compose stack serves today has `refs/remotes/*` populated, and
  nothing in either produces one.** `git-init`/the post-receive hook only ever write `[cgit]`
  metadata and the agefile; every repository arrives via SSH push, never `git remote add` +
  `fetch`. Searched the whole tree (docs, scripts, fixtures, tests, git log) for "remote"/"mirror"/
  "upstream" — the only git-remote-sense hits were the ROADMAP gap bullet itself. Also worth
  recording: `git clone --mirror` (the literal reading of "mirror repository") lands upstream
  branches in `refs/heads/*` via its `+refs/*:refs/*` refspec, so a `--mirror`ed repository was
  *already* fully visible through the existing local-branch path — `refs/remotes/*` only appears
  from a hand-configured `remote add` + custom fetch refspec against a bare repo, a genuinely
  unusual manual setup. `docs/API.md` states this plainly so a future reader doesn't wonder why
  nothing populates the field.
- **Built anyway, scoped conservatively** given the above: a separate `RefsInfo.remote_branches:
  Vec<BranchRef>` (reusing the existing type — git2 exposes no way to split a remote branch's name
  from its remote, e.g. `"origin/main"` is the whole shorthand), not merged into `branches` — that
  would have silently changed what `RepoSummary.branch_count` counts.
- **`refs.rs::branches()` generalized into `branches_of_kind(repo, BranchType)`**, shared by both
  `branches()`/`remote_branches()` — the iterator's kind field was already there, just discarded
  with `_`.
- **A remote's own symbolic `HEAD` (e.g. `origin/HEAD`) is skipped.** libgit2's remote-branch
  iteration selects purely on the `refs/remotes/` prefix and doesn't exclude it, and it peels to a
  commit just fine, so the existing "unresolvable tip" guard wouldn't catch it — it would otherwise
  show up as a redundant alias row for whichever branch that remote's default actually is.
- **Web only gets Log and Compare links, not Tree.** `?ref=`-driven endpoints (log, diff, stats,
  search) already resolve `origin/main`-shaped names correctly — `resolve_commit`'s
  `revparse_single` follows libgit2's own DWIM rule that tries `refs/remotes/%s`, and the query
  param carries the value already URL-decoded. Tree/blob/blame are different: their `{ref}/{path...}`
  wildcard route depends on `resolve.rs::ref_shorthands()` to find the ref/path boundary, and that
  set only ever collected `refs/heads/`/`refs/tags/` shorthands. Extending it to remote branches
  raises real questions (a local branch named `origin` coexisting with a remote branch
  `origin/main`, disambiguated only by longest-match) that aren't worth resolving for a feature with
  no confirmed user — left as a candidate.
- **The section renders nothing at all when `remote_branches` is empty**, unlike Branches/Tags'
  always-present "No branches."/"No tags." — showing "No remote branches." permanently on every
  repository page would be pure noise for a feature essentially nothing exercises yet.
- **`RefBadges`/`useCommitRefs` and `DiffView`'s revision datalist are untouched** — same reasoning
  as #51's `RefBadges` non-change: a local `main` and `origin/main` pointing at the same commit
  would just double the badge for no new information, and the datalist is decoration a typed
  `?from=&to=` value doesn't need.
- Test fixtures use `git update-ref refs/remotes/{name} {target}` directly — no real `git remote
  add`/`fetch` involved, matching the finding above that this is a hand-configured shape, not
  something to simulate a whole second repository for.

## #53 Object links for non-commit refs (`GET /objects/{oid}`, cgit's `cgit_object_link()` parity)

Closed the last "Tags and refs" cgit-parity gap, left open on purpose by #51: a tag's target that
isn't a commit had a known kind (`TagDetail.object.type`) but nowhere to link to — axgit's
tree/blob/raw routes are all ref+path based, with no by-oid equivalent anywhere in the API. Also
closed the separate "Fetching a blob directly by object id" gap under "Tree and blob" — one
feature, two gap bullets. Landed as five commits: `GET /refs` learning the target kind, the by-oid
detail endpoint, the by-oid raw endpoint, the web object page, then wiring the two existing
non-commit-target call sites (`RefsView`, `TagView`) onto it.

- **`TagRef` gains `object: { sha, type }`, and `target` becomes nullable** — `GET /refs` used to
  fall back to the *peeled object id* for a tag that never reaches a commit, writing a non-commit
  oid into the same field a commit sha goes into; `RefsView.tsx` then rendered it under a column
  literally headed "Commit". That fallback is deleted; `object` is now the same one-level
  dereference `GET /tags/{name}` already reports (same name, same shape, same meaning, per #51's
  "one word, one meaning" rule), and `target` keeps `GET /tags/{name}`'s existing nullable meaning
  instead of a separate one. The dereference logic itself is shared via a new
  `tag::dereference` helper (extracted from `tag::detail`) so the two endpoints structurally cannot
  drift apart again.
- **`GET /objects/{oid}` requires a full 40-character hex id — abbreviations are `400
  invalid_param`.** Every link axgit itself emits already carries a full oid, and requiring one is
  what lets the endpoint be **unconditionally immutable-cached**: unlike every other tag/branch-
  shaped URL in this API, the address here *is* the content, so there's no validator to check.
  `repo::object::parse_full_oid` rejects on length or non-hex bytes before touching the repository
  or the cache.
  - **One envelope, not a `oneOf` union** (`ObjectDetail { sha, type, tree, blob, tag }`, only the
    `type`-matching payload non-null): keeps this document's "every key always present" invariant
    rather than special-casing this one endpoint. A `commit`-typed object carries no payload at
    all — the web view links straight to `GET /commits/{sha}` instead of duplicating that data
    here.
  - **`TreeEntryInfo` gains `sha`** (additive on `GET /tree` too) so a tree's entries can link
    onward by oid; the entry-building loop (`repo::tree::list_tree`) is now shared via a new
    `entries_of` helper between the ref+path tree endpoint and the by-oid one, so the two can't
    disagree on ordering or field shape either.
  - **`repo::tag` gains a second, parallel dereference helper (`dereference_object`)** rather than
    reusing `dereference`: the shared one takes a `git2::Reference` and calls
    `Reference::peel_to_commit()` for `target` (a single native libgit2 call that already walks
    nested tags), but a tag reached by its own oid has no reference behind it — only a resolved
    `git2::Tag` — so `dereference_object` walks to the peeled commit via `Object::peel` instead.
    Both are thin wrappers over the same libgit2 peeling behavior; kept as two functions rather than
    one accepting either input, since unifying them would mean threading a `Reference`-or-`Object`
    enum through code that's otherwise identical either way.
  - **`ApiError::ObjectNotFound` (`404 object_not_found`) is a new variant**, not a reuse of
    `RefNotFound`: there's no ref or sha *resolution* involved in a by-oid lookup, just a direct
    object-database miss — conflating the two would blur a distinction every other 404 in this API
    already draws (`path_not_found` vs. `ref_not_found`).
- **`GET /objects/{oid}/raw`** is the by-oid analogue of `GET /raw/{ref}/{path...}`: blob bytes
  only (`404 object_not_found` for any other kind), always immutable, always
  `X-Content-Type-Options: nosniff`. With no filename behind an oid there's no extension to guess a
  `Content-Type` from, so it's always `text/plain; charset=utf-8` or `application/octet-stream` —
  never `mime_guess`.
- **Web: `/{repo}/object/{oid}`, a new placeholder-shell route** (`shellFor`/`shell_for` both gain
  an exactly-3-segment arm, same shape as `commit`'s — an oid never contains `/`), not a `RepoNav`
  tab: `RepoLayout`'s nav highlighting maps it to "Refs", the same choice #51 made for the tag
  page, since both are drill-downs reached from Refs/tag pages rather than places a user browses to
  directly.
  - **Tree navigation inside the object page is oid-to-oid, with no path context** — there's no
    root commit behind a bare oid to build a breadcrumb or a `path` from, so `ObjectView.tsx`
    doesn't try to fake one; each entry just links to another `GET /objects/{oid}` call by its own
    `sha`. A gitlink entry stays inert text, same as `TreeView.tsx`'s submodule row — its sha names
    a commit in another repository, unreachable through this one.
  - **A blob's `CodeBlock` gets `path=""`** — there's no filename behind an oid for Shiki to guess
    a language from, so it renders as plain text, the same fallback an unrecognized extension
    already gets elsewhere.
  - **A tag's non-commit dereference links onward through the object page too** (not just
    `RefsView`/`TagView`'s existing rows) — this is what actually lets a nested tag be followed one
    hop at a time; without it, the object page itself would dead-end exactly where the gap started.
  - **`Disallow: /*/object/` added to `robots.txt`**: the object graph is walkable link-by-link —
    cheap per request, but not something worth handing a crawler, matching the existing scan-
    budgeted-endpoint rule (`search`/`stats`/`blame`/`diff`).
- **`RefsView.tsx`'s tag Object cell and `TagView.tsx`'s Object row both route by `object.type`**:
  `commit` → `commitHref` (unchanged), anything else → the new `objectHref`. This is the change that
  actually closes the gap — #51 had already narrowed it by making the *kind* knowable
  (`TagDetail.object.type`); until this commit neither call site acted on a non-commit kind, so a
  tree/blob/nested-tag target still rendered as inert text with nowhere to go.
- **Fixtures gained a tag on a tree** (`tree-tag`, alongside #51's existing blob tag) so the by-oid
  tree case — not just the blob case — is reachable in a local run (`scripts/make-fixtures.sh`).

## #54 Archive format coverage (`tar.bz2`/`tar.xz`/`tar.zst`, cgit's snapshot format parity)

Closed the "Archive" cgit-parity gap: cgit offers `tar`, `tar.gz`, `tar.bz2`, `tar.lz`, `tar.xz`,
`tar.zst`, `zip` by piping `git archive --format=tar` through an external compressor binary; axgit
had only `tar.gz` and `zip`, the two formats `git archive` produces natively.

- **In-process streaming encoders (`async-compression`), not external compressor binaries, and not
  git's own `tar.<fmt>.command` config.** cgit's approach — forking `bzip2 -c`/`xz -c`/`zstd -c` —
  would mean adding those packages to the runtime image and assuming they exist on every dev/CI
  host too. `git archive --format=tar` output is instead wrapped in a
  `tokio::io::BufReader`-backed `async_compression::tokio::bufread::{BzEncoder,XzEncoder,
  ZstdEncoder}` and streamed the same way `tar.gz`/`zip` already were, so `Body::from_stream` +
  the existing reaper/`kill_on_drop` machinery need no change, and #14's "exec only ever sees a
  resolved sha" invariant is untouched — the encoder sits *after* exec, not in it. The Alpine
  runtime image stays `git` + `ca-certificates` (docs/ARCHITECTURE.md).
  - `bzip2`'s and `zstd`'s Rust backends (`libbz2-rs-sys`, bundled `zstd-sys` C) build with no
    extra system dependency. `xz`'s backend (`liblzma-sys`) is pinned via a direct `liblzma = {
    features = ["static"] }` dependency — without it, `liblzma-sys` pkg-config-probes for a system
    liblzma and links it dynamically when one happens to be installed (e.g. Homebrew's `xz` on a
    macOS dev machine), making the linked xz version depend on the build host, the exact class of
    problem `libgit2-sys`'s vendored build already avoids for libgit2 itself. `static` forces the
    same vendored-C-via-`cc` posture everywhere, and the `musl-dev` package the api build stage
    already installs for `libgit2-sys` covers it.
- **Format table replaces the three-armed `format_arg`/`content_type`/`extension` match.**
  `handlers/archive.rs`'s `ArchiveFormat` went from a two-variant enum to a `FORMATS: &[struct]`
  table (suffix, `git archive --format` value, media type, optional encoder) that both
  `parse_archive_target` and the response builder read from — adding a format is now a one-line
  table entry rather than touching three separate match expressions that could silently drift out
  of sync. `parse_archive_target` matches any suffix in the table (stripping the format suffix and
  its separating `.`) rather than trying a fixed order, since none of the five suffixes is itself a
  suffix of another.
- **Plain `tar` and cgit's `tar.lz` are deliberately not offered.** An uncompressed multi-megabyte
  download is a poor default over HTTP with no real use case distinct from `tar.gz`; `tar.lz`
  (lzip) has no maintained Rust encoder, so matching it would mean reintroducing exactly the
  external-binary dependency this decision avoids for the other three. `main.tar` stays a `400
  invalid_param`, same as before this change — a regression test pins that.
- **Compression levels are pinned to constants matching cgit's own CLI defaults**
  (`bzip2 -9`, `xz` preset `6`, `zstd -3` — cgit passes no level flag, so each tool's default
  applies), not left at `async-compression`'s `Level::Default`: reading the crate source shows
  `Level::Default` is bzip2 `6` and xz preset `5`, not the CLI defaults, so leaving it would have
  quietly served weaker compression than cgit at the same format.
- **A bounded `Arc<Semaphore>` (`AppState::archive_encoder_limit`, 4 permits) gates only the three
  encoder formats**, acquired before `git archive` is even spawned so an over-capacity request
  waits rather than spawning a process it isn't ready to read from. Archives are never
  response-cached (`cache_test.rs`), so every request builds a fresh encoder; an xz preset-6
  encoder alone holds on the order of 90 MiB, and cgit's identical exposure was masked by each
  request being a separate forked process — here it's the same server process, so unbounded
  concurrency is a real memory/CPU risk this repo didn't previously have. `tar.gz`/`zip` requests
  never touch the semaphore, since git already did their compression. The permit is held by a small
  `Guarded<R>: AsyncRead` wrapper that owns it for the response body's lifetime, released on drop
  (client disconnect or stream completion) rather than after headers are sent.
- **`Content-Type` follows the same "prefer a registered media type" rule the existing `tar.gz`/
  `zip` values already set**: `zstd` has an IANA-registered type (`application/zstd`, RFC 8878
  §7.1) and uses it; `bzip2`/`xz` have none, so they use the de facto `application/x-bzip2`/
  `application/x-xz` (matching cgit's own choice).
- **ETag stays repo-scoped, not per-format.** `validator_etag` is unchanged (HEAD sha + agefile
  mtime), and HTTP caches key on the full URL, so `main.tar.gz` and `main.tar.zst` never collide —
  worth stating explicitly so a future reader doesn't "fix" a perceived gap by folding the format
  into the validator.
- **A mid-stream `git archive` failure now produces a well-formed-but-truncated file for the three
  encoder formats**, not a detectably-broken one: today's `tar.gz` failure yields an invalid gzip
  stream a decompressor rejects outright, but an encoder still finalizes its own container around
  whatever truncated tar bytes it received. `tar` itself still catches the missing end-of-archive
  blocks; a naive consumer might not. Documented in `docs/API.md` rather than treated as a defect,
  since fixing it would mean buffering the entire archive before sending any bytes.
- **Tests decompress with the same crate the handler encodes with, not a system `bzip2`/`xz`/
  `zstd` binary.** macOS's bsdtar doesn't reliably support `zstd`, and GNU tar shells out to
  external binaries for `-j`/`-J`/`--zstd` rather than decoding in-process — neither is a safe
  assumption about the test host, so `archive_test.rs` round-trips through
  `async_compression::tokio::bufread::{BzDecoder,XzDecoder,ZstdDecoder}` (a dev-dependency) and
  additionally checks each format's magic bytes, which a matching decoder alone wouldn't catch if
  the *container* were wrong. The original `tar -xzf` extraction test is kept as-is for `tar.gz`,
  which remains safe to test against the system tool everywhere.
- **Web**: `web/src/lib/api/repos.ts` gained a single exported `ARCHIVE_FORMATS` const (with
  `ArchiveFormat` derived from it via `(typeof ARCHIVE_FORMATS)[number]`), replacing three
  hardcoded `tar.gz`/`zip` pairs across `RepoSummary.tsx`, `RefsView.tsx`, and `TagView.tsx`. All
  three now render every format. `RefsView.tsx`'s tags-only download-column rationale (#51) is
  unaffected — it's about which *refs* get a download, orthogonal to which formats are offered.

## #55 Archive download links on the commit page (closes the last "Commit page" cgit-parity gap)

Closed the last item under "Commit page": the Tree link, per-parent `(diff)` links, and
patch/rawdiff links were already wired by the diff/patch work (#38/#39), but the commit detail
page had no archive downloads while the summary, refs, and tag pages all did (#51, #54). Web-only,
one commit — `GET /repos/{repo}/archive/{ref}.{format}` already accepts any ref including a full
sha, so no API change was needed.

- **The commit's own sha is the ref, not the URL's `{sha}` param.** `CommitView.tsx` passes
  `archiveUrl(resolvedRepo, detail.sha, format)` — `detail.sha` (the resolved response field, always
  the full 40-character id) rather than `resolvedSha` (whatever the visitor typed into the URL,
  possibly abbreviated). This is also what keeps the request on `handlers/archive.rs`'s
  `immutable = refname == sha` path, matching every other full-sha-addressed thing on this page
  (the commit detail/diff responses themselves).
- **This inverts #51's branch-exclusion rationale, on purpose.** #51 kept the tags-only download
  column off branches because a branch archive's filename names a moving target that changes
  meaning on every push. A commit sha is the opposite case — the most reproducible address
  `archiveUrl` can be given, immutable-cacheable by construction.
- **Rendered inline in the existing action row** (`Tree | Raw diff | Patch`), appending the five
  `ARCHIVE_FORMATS` links after `Patch` — the same plain-text-link markup `TagView.tsx` already
  uses for its Tree/Log/format row, not `RefsView.tsx`'s per-row `aria-label` variant. The
  `aria-label` override exists there because the same link text (`tar.gz`) repeats down a column of
  tag rows; the commit page, like the tag page, renders one set per page, so the format name alone
  is already the link's full accessible name.
- **No gating condition**, unlike `TagView.tsx`'s `canBrowse` (which exists because a tag can point
  past a commit, at a tree/blob, and `git archive` needs a treeish). A commit always has a tree, so
  every commit detail page renders all five links unconditionally.
- **Diff display options (`view`/`context`/`ignorews`/`path`) are not threaded into the archive
  links.** They control how the *diff* against the first parent is rendered; the archive is always
  the commit's full tree, independent of those params.
- **No shared component extracted** for the four call sites (`RepoSummary`'s icon `MetaLink`,
  `RefsView`'s table cell + `aria-label`, `TagView`'s and now `CommitView`'s inline text). Each
  wraps the links differently enough that a shared component would just grow option props; the
  actual duplication — the format list — is already centralized in `ARCHIVE_FORMATS` (#54).

## #56 Rename following in the commit log's path filter (`follow=1`, cgit's `enable-follow-links`)

Closed the first of the two remaining "Log" cgit-parity gaps. Without this, `path=`'s history
simplification (`commits.rs::touches_path`, entry-id comparison against every parent) treats a
rename as "the old path stopped existing, the new path started existing" — correct in isolation,
but it means the log for a renamed file's new name silently ends at the rename instead of
continuing into its history under the old name, the way `git log --follow` and `repo::blame`
(#36) both do.

- **`commits::log` grew a `LogParams` struct** (`path`/`skip`/`limit`/`include_body`/`follow`),
  replacing five positional arguments — the same "one params struct" shape `diff::DiffParams`
  already uses, and needed now that a sixth parameter (stat counts, see the next candidate item)
  was already on the horizon.
- **The walk tracks a mutable `tracked: Option<PathBuf>`, reseeded from `path` on every call**,
  not resumed from anywhere — consistent with the walk itself always restarting from `start`
  rather than a boundary commit (#37's cursor design). This is what keeps a `follow=1` cursor page
  lossless the same way a plain `path=` one is: every page re-derives the same rename chain from
  scratch, so there is nothing page-boundary-dependent to get out of sync.
- **Rename detection only runs where a rename could plausibly be** — when `path_entry_id` shows
  the tracked path present in the commit but absent from its first parent (`first_parent_lacks_path`,
  new in `commits.rs`). An ordinary add or modify never reaches the (relatively expensive) full
  first-parent-tree diff this guards; a `diff::rename_source` lookup (new in `diff.rs`) only runs
  for the shape a rename actually has.
- **`diff::rename_source` diffs the *whole* first-parent tree, not a pathspec-restricted one** —
  unlike every other `build_diff` caller in that file. A rename's old path can't be predicted, so
  there is nothing to restrict the pathspec to; it reuses `find_similar`'s libgit2 defaults (same
  50% similarity threshold as the diffstat/diff endpoints) to actually find the match.
- **Only `Delta::Renamed`, not `Delta::Copied`, is followed** — matching `repo::blame`'s existing
  "only whole-file renames are tracked" rule (its `orig_path` field), so the two history-following
  features agree with each other. cgit's own `--follow`-equivalent doesn't follow copies either.
- **`MAX_FOLLOW_RENAME_LOOKUPS = 100`** bounds how many of these full-tree diffs a single walk can
  attempt, same scan-budget rationale as `Cursor::MAX_OFFSET` and search/stats' budgets
  (#26/#28). Past the cap, the walk keeps filtering on whatever path it was last tracking rather
  than erroring — a rename chain that long is already far outside any real usage this endpoint
  sees.
- **`follow` is silently a no-op without `path`**, both in the api (the whole tracked-path branch
  of the walk is skipped when `path` is `None`, so the flag is never even read) and in
  `CommitLog.tsx` (`following` is computed as `Boolean(resolvedPath) && resolvedFollow === "1"`,
  so a stray `?follow=1` with no `path=` never reaches `listCommits` or renders the toggle). No
  `400` — cgit's own `enable-follow-links` has nothing to be invalid about either, it just does
  nothing without a single-file path.
- **`CommitInfo` gained `renamed_from: Option<String>`, key omitted (not `null`) when absent** —
  same convention `body` established (#44): off by default, present only on the one entry that is
  the renaming commit itself. `commit_info()`/`commit_info_with_body()` both default it to `None`,
  so `/search`'s reuse of `commit_info()` is untouched — search has no `follow` concept and never
  sets the field.
- **`follow` joined `msg` in the cache key's `params` string** (`handlers/commits.rs`) — same
  reasoning as #44: it doesn't change which walk-start is cache-relevant (a fixed start makes
  `follow` a pure "which commits/fields come back" selector, just like `path`/`limit`/`msg`), but
  it does change the response body, so a `follow=1` response must not alias onto the non-follow
  entry for the same request.
- **Web**: `CommitLog.tsx`'s existing path-filter banner gained a `Follow renames`/
  `Stop following renames` URL-only toggle next to `clear filter` — same "display option lives in
  the URL" rule `msg=1` (#44) and `stat=1` (#43) established, preserving `ref`/`cursor`/`msg` (and
  vice versa: the `Expand messages` toggle now preserves `follow` too, and `Older →` carries both).
  A row whose commit carries `renamed_from` shows `renamed from <old path>` next to its `RefBadges`.
- `docs/API.md`/`docs/openapi.json`/`web/src/lib/api/types.ts` updated (`GET /commits` gained
  `follow`, `CommitInfo` gained optional `renamed_from`).

## #57 Files/Lines changed columns on the commit log (`stat=1`, cgit's `enable-log-filecount`/`enable-log-linecount`)

Closed the second and last "Log" cgit-parity gap. cgit ships these as two independent flags; axgit
merges them into one `stat=1` because there is no per-repo display config here to keep them
separate for (`cgit.section`/`.owner`/`.desc` in the repo's `config` is metadata, not a rendering
toggle) — splitting them would only double the cache-key value space for no real use case.

- **`diff::stat_counts` is a new, cheaper sibling of `diffstat`** — one `Diff::stats()` call
  (`files_changed()`/`insertions()`/`deletions()`) instead of a `Patch::from_diff` per file.
  `diffstat` needs the per-file breakdown (rename pairs, per-file binary flag, the full
  `DiffStatFile` list) for the commit detail page; the log column needs none of that, just three
  totals, once per row — the per-file loop would have been wasted work multiplied by `limit`.
  Both still share `commit_trees`/`build_diff`, so both stay first-parent by the same construction,
  including for merges.
- **Restricted to the log's own `path` filter, using the same snapshot `follow` (#56) introduced**
  — `commits::log` now clones `tracked` into `filter_path` at the top of each loop iteration,
  *before* a rename crossing can mutate `tracked` for older commits, and passes that snapshot to
  both `touches_path`/`rename_source` and (new) `stat_counts`. Getting this ordering wrong — using
  the post-mutation `tracked` — would make the renaming commit's own stat count against its *old*
  name instead of the name `touches_path` just filtered it in under.
  - **This restriction is `path`-only, not rename-aware**, unlike the diffstat/diff endpoints: a
    pathspec restricts the *tree diff itself* before `find_similar` ever runs, so on the renaming
    commit `path=<new name>&stat=1` sees the new file as a plain addition (old side, `<old name>`,
    doesn't match the pathspec and is dropped from the diff before pairing) rather than as the
    zero-change rename the unrestricted diffstat would show. Documented in `docs/API.md` rather
    than special-cased — matching a real `git log --stat -- <path>` restriction, which has the
    same property.
- **`LogParams` gained `include_stat: bool`**, the second field to join it after `follow` (#56) —
  the struct's docs/DECISIONS note predicting this ("a sixth parameter... already on the horizon")
  turned out right one commit later.
- **`CommitInfo` gained `stat: Option<StatCounts>`, key omitted (not `null`) when absent** — same
  convention `body` (#44) and `renamed_from` (#56) both established. `commit_info()` defaults it to
  `None`; `/search`'s reuse of `commit_info()` is unaffected, same as `renamed_from`.
- **`stat` joined `follow`/`msg` in the cache key's `params` string** — same reasoning as both:
  irrelevant to the immutability decision (a fixed walk start makes it a pure field-selector, like
  `path`/`limit`), but it changes the response body, so it can't share a cache entry with the
  default response.
- **Web**: `CommitLog.tsx`'s `Expand messages`/`Collapse messages` link gained a sibling `Show
  changes`/`Hide changes` toggle on the same line (URL-only, `stat=1`, preserving every other
  param — same `msg=1` (#44)/`follow=1` (#56) precedent), plus two right-aligned `Files`/`Lines`
  columns rendered only when `stat` is on, formatted `+{additions} −{deletions}` to match the
  existing diffstat UI (`DiffFile.tsx`). Unlike `msg`/`follow`, `stat` doesn't depend on a `path`
  filter — it's a plain per-repo log preference, so the toggle always renders once there are
  commits to show. The expanded-message row's `colSpan` (previously hardcoded to `4`) became a
  computed `columnCount` (`4 + (showStat ? 2 : 0)`) so the two extra columns don't leave it short.
- `docs/API.md`/`docs/openapi.json`/`web/src/lib/api/types.ts` updated (`GET /commits` gained
  `stat`, `CommitInfo` gained optional `stat`, new `StatCounts` schema).

## #58 Hex dump view for binary blobs (cgit's `<table class='bin-blob'>` parity)

Closed the "Hex dump view for binary blobs" gap under "Tree and blob" — until now `BlobView.tsx`
and `ObjectView.tsx` both dead-ended a binary blob at "Binary file not shown — view raw", which
made the binary-blob page itself pointless (magic numbers, embedded strings, small icons all
invisible without downloading).

**Web-only, no API change.** The bytes come from the raw endpoint the blob/object pages already
link (`rawUrl`/`objectRawUrl`), fetched client-side rather than added as a new field on
`BlobInfo`/`ObjectBlob` — `content: null` there already means "fetch raw instead," and a base64
copy of the same bytes in the JSON response would bloat every binary blob response by a third for
no reader that doesn't already have the page open.

- **16 bytes per row, not cgit's 32** (`web/src/lib/format/hex.ts::HEX_BYTES_PER_ROW`) — matches
  `xxd`/`hexdump -C` convention and keeps `offset + 3×16 hex chars + 16 ascii` narrow enough to
  avoid horizontal scroll on a phone-width viewport; cgit's own width was tuned for a desktop-only
  CGI era. Rendered immediately when `binary: true`, not behind a toggle — same as cgit, and
  consistent with everything else on the blob page (content, symlink target) rendering
  unconditionally.
- **Two independent boundaries, each reusing an existing number rather than inventing one**:
  the *fetch* is gated on `!blob.too_large`, i.e. the api's existing 1 MiB `BLOB_CONTENT_LIMIT` —
  an over-1-MiB binary keeps the pre-existing "File too large" notice, never triggering a raw
  fetch at all. The *render* is separately capped at `HEX_DUMP_LIMIT = 64 KiB` (4096 rows) with a
  visible "Showing the first 64.0 KiB of N — view raw" note, the same shape as the diff endpoint's
  1000-line-per-file cap (#38) — a binary file under 1 MiB can still be tens of thousands of DOM
  rows at 16 bytes/row, and nothing about a hex dump past the first few KiB is usually worth
  rendering anyway (raw download covers the rest).
- **`web/src/lib/format/hex.ts::hexRows`** is a pure layout function (the `commit-graph.ts`/
  `markdown-url.ts` precedent — layout logic lives outside the component, unit-tested on its own).
  Truncates to `HEX_DUMP_LIMIT` before splitting into rows; each row carries the offset, one 2-char
  hex string per byte (not a single joined string), and a parallel ASCII string (`0x20`-`0x7e`
  passed through, everything else `.`) — leaving the hex/ASCII string formatting (group spacing,
  padding a short final row for column alignment) to the component, same split as `CommitGraph.tsx`
  rendering `commit-graph.ts`'s pure geometry.
- **New `web/src/lib/api/repos.ts::fetchRawBytes`** — the first client-side consumer of a raw
  endpoint's actual bytes rather than just its `href` (every other `rawUrl`/`objectRawUrl` caller
  only builds a link). Lives beside `rawUrl` rather than in `client.ts`, since `apiFetch` is
  JSON-only by construction (`Content-Type`/error-envelope parsing that doesn't apply to a binary
  body) and keeps `API_BASE` module-private; `fetchRawBytes` instead takes the full URL a
  `rawUrl`/`objectRawUrl` call already produced. Same `ApiError` contract as `apiFetch`
  (network failure and non-`ok` both throw), so `HexDump`'s fetch/catch needs no special-casing.
- **New `web/src/components/repo/HexDump.tsx`** — `{ url, size }`, the `loading | error | data` +
  `cancelled`-flag state machine every other island uses. A failed fetch renders the *same*
  "Binary file not shown — view raw" notice the blob/object pages used to show unconditionally, so
  a network error degrades to the old behavior instead of a blank panel. Wired into both
  `BlobView.tsx`'s and `ObjectView.tsx`'s `binary` branch — the two were identical dead ends before
  this, so both got the swap in one pass rather than leaving one page behind.
- `scripts/make-fixtures.sh` gained a small (520-byte) binary file with a NUL-containing byte
  pattern in `git-compose.git`'s "docs: add notes" commit, written byte-by-byte with `printf` for
  reproducibility — the classify/hex-dump path was otherwise unreachable in a local run (nothing
  in the existing fixtures has a NUL byte; extension alone doesn't trigger `Blob::is_binary()`).
- No `docs/API.md`/`docs/openapi.json`/`web/src/lib/api/types.ts`/`api/**` change.

## #59 Stats API: `path=` filter (cgit `ui-stats.c`'s `ctx.qry.path` parity)

Closed the last item under the `docs/ROADMAP.md` "Stats" cgit-parity gap: `/stats` only ever
aggregated the whole repository, so there was no way to answer "who's been committing to this
directory" the way cgit's stats page can. Same two-commit shape as #46/#47 (search's author/
committer/range types) — API first, web page as a follow-up (#60).

- **Reuses `repo/commits.rs::touches_path` verbatim, promoted to `pub(crate)`**, rather than
  writing a second path-filter predicate. It's the same question ("did this commit change this
  file/directory relative to its parents") the commit log's own `path` filter already answers, and
  the two call sites drifting into slightly different merge-commit approximations would be a worse
  outcome than one shared function with two callers.
- **No `follow` support.** cgit's stats page doesn't track renames either, and `/commits`'
  `follow=1` machinery (#56) pays for a full first-parent tree diff on every rename-shaped commit —
  a cost the search/stats budget family (#26/#28) was built to bound, not add to. Left as a ROADMAP
  candidate rather than gap: cheap to add later by threading the same `tracked`-path mutation
  `commits::log` already has, if a real need shows up.
- **Filter applied after the bucket-window check, not before.** The revwalk loop already discards
  commits outside the fixed 12-bucket window via `bucket_index`; checking `touches_path` afterward
  means a commit that's simply too old never pays for the (relatively expensive) per-parent tree
  lookup. `MAX_SCANNED_COMMITS` is unaffected either way — it counts commits the revwalk visits,
  not commits that pass the filter, so `truncated`'s meaning doesn't change.
- **A path that never existed returns `200` with every bucket at `0` and `author_count: 0`, not a
  `404`** — the same carve-out `/commits?path=` already documents, kept consistent rather than
  reintroducing a 404 case stats didn't have before.
- `docs/API.md`/`docs/openapi.json`/`web/src/lib/api/types.ts` updated (`GET /stats` gained `path`,
  documented with the same wording as the commit log's own `path` field). The `/{repo}/stats` page
  surfacing it is a follow-up commit (#60).

## #60 `/{repo}/stats` page: surface the `path` filter

The web half of #59 — API-then-page, same order #46/#47 and #59 itself followed.

- **Entry point is a `Stats` quick link on each tree row (`TreeView.tsx::rowActions`), not the nav
  tab.** cgit reaches a path-scoped stats view by having its `stats` nav tab carry the current
  `ctx.qry.vpath` — but axgit's tab bar (`RepoNav.astro`) is static HTML prerendered once under a
  placeholder param (DECISIONS #17) and filled in per-navigation by
  `window.__axgit.fillRepoShell`, which has no notion of "the tree page's current path." Extending
  that mechanism for one tab, on one page, would be a lot of shell-filling machinery for what #48
  already solved the general case of: a per-row action link built from data the row already has.
  `rowActions` gained a fourth entry (`Stats`, `ChartBarIcon`) alongside Log/Raw/Blame, using the
  same `statsHref(repo, { path, ref })` shape `logHref`/`blameHref` already follow — available on
  both file and directory rows (matching `path`'s file-or-directory rule), absent on submodule rows
  (which get no row actions at all, per #48).
- **`StatsView.tsx` resolves `path` with the same prop-overrides-`location` pattern every other
  param uses** (`pathParam ?? paramFromSearch(...)`), carries it through every `statsHref` the
  period switcher builds (so switching `week`/`month`/`quarter`/`year` doesn't drop the filter),
  and renders a `Filtered by path … — clear filter` banner directly modeled on `CommitLog.tsx`'s —
  minus the "Follow renames" link, since #59 deliberately didn't add `follow` to the endpoint.
- No route change (`/stats` already takes query params only, `shellFor`/`shell_for` untouched), no
  API contract change — `StatsView.tsx`, `TreeView.tsx`, `lib/api/repos.ts::StatsParams`, and
  `lib/repo-href.ts::statsHref` only.

## #61 Atom feed gains `ref`/`path`/`all`/`limit`

Closes the last open "Feed and discovery" cgit-parity gap (`h=`/path filter/`all=1`/
`max-atom-items`). Same two-commit shape as #46/#47 and #59/#60 — API first, web surfacing as a
follow-up (#62).

- **`?ref=`, never cgit's `h=`.** #18 already settled this for the whole API — "ref selection is
  `?ref=`" — and #35 treats `h=` as a redirect-only legacy alias, not a spelling to adopt anywhere
  new.
- **`all=1`'s scope is `refs/heads/*` + `refs/tags/*` only**, matching the ref-shorthand resolver's
  existing definition of "a ref axgit can resolve." Not `refs/remotes/*` — #52's own investigation
  found no fixture or real repository that actually populates remote-tracking branches, so there's
  nothing there to walk yet; extending the scope later is additive, not a breaking change to the
  canonical query.
- **The multi-tip walk uses `git2::Revwalk::push_glob` on the two globs**, not a manual
  `repo.references()` loop. libgit2 peels an annotated tag to its target commit and silently skips
  any ref that doesn't peel to one (verified directly:
  `log_all_refs_should_skip_a_ref_that_does_not_peel_to_a_commit` tags a blob and asserts the walk
  doesn't error), so no extra filtering is needed on axgit's side. Two explicit globs — not
  `refs/*` — keep `refs/remotes` and `refs/notes` out by construction rather than by an exclusion
  check.
- **`Sort::TIME`, but only on the multi-tip walk.** The single-tip `log()` stays unsorted
  (libgit2's default DFS from the pushed tip), deliberately — with one tip that's the closest match
  to `git log`, and changing it would desync the feed's ordering from the Log tab for no benefit.
  With several unrelated tips, though, the same DFS drains one tip's ancestry before touching the
  next: a stale tag pushed first would starve the walk of every other branch's recent commits, and
  `<updated>` (taken from the first entry) would report that stale date forever — a feed that looks
  permanently frozen. `git log --all`'s own default is date order for the same reason. This is
  deliberately *not* `diff.rs::format_patch`'s `Sort::TOPOLOGICAL | Sort::TIME`: that combination
  exists to keep a parent from appearing after its child in a patch series, which re-introduces
  exactly the branch-grouping the feed is trying to avoid. One caveat worth recording: libgit2
  sorts on **committer** date while `<updated>` reports **author** date, so a rebased/imported
  history can still produce a non-monotonic `<updated>` sequence — accepted, since Atom readers
  sort by `<updated>` themselves and cgit has the same property.
- **`repo/commits.rs::log()` split into itself plus a private `collect()`** that takes an
  already-pushed `Revwalk` and does everything past that (path filter, rename following, skip/limit,
  `CommitInfo` building). `log()` keeps constructing the single-tip walk and encoding the `Cursor`
  from that one `Oid`; the new `pub fn log_all_refs()` builds a `Sort::TIME` multi-glob walk and
  calls `collect()` too, returning `Vec<CommitInfo>` with no cursor at all. Deliberately **not** a
  `LogStart { Oid(Oid), AllRefs }` enum threaded through `log()`: a multi-tip walk has no single
  start to encode a cursor from, and `log_all_refs`'s cursor-less return type makes that a fact of
  the type system instead of a branch `log()` would have to reject at runtime.
- **`ref` is ignored when `all=1` is set**, the same way the commit log's `cursor` already makes it
  ignore `ref` — computed once as `effective_ref` before the cache key, the canonical query, and the
  walk all read it, so `?all=1&ref=nope` never resolves `nope` at all (no `404`) and shares one
  cache entry with `?all=1`. A `400` was considered and rejected: a bookmarked feed URL with a since-
  deleted `ref` would otherwise poll a permanent error forever, and `ref` genuinely has no effect on
  an `all=1` walk.
- **No `follow` support**, the same call #59 made for stats: cgit's own Atom view doesn't track
  path filters across renames either, and `follow`'s rename-lookup cost (#56) isn't worth adding to
  an endpoint with no pagination to spread it across.
- **`<id>`/`rel="self"` carry a canonical query string**, generated from the *parsed* params
  (`canonical_query`) rather than echoed from the request, so `?path=/src/`, `?path=src`, and
  `?limit=20&path=src` all resolve to the same feed identity — RFC 4287 §4.2.6 requires a feed's
  `<id>` to be stable and to uniquely identify that feed, and with parameters, `/feed.atom?ref=dev`
  and `/feed.atom` really are different feeds. Fixed param order (`all`, `ref`, `path`, `limit`)
  regardless of request order; a param at its default is omitted entirely, so the all-defaults
  feed's `<id>` is byte-identical to the pre-#61 unparameterized one. Entry `<id>`s stay
  `urn:sha1:{sha}` — that's cross-feed, cross-host commit dedup, a different problem from a feed's
  own identity, so the two need not (and don't) share an encoding scheme.
- **No immutable caching, even when `ref` resolves to a full sha.** The mechanical test used
  elsewhere (`stats.rs`'s `query.r#ref.as_deref() == Some(sha.as_str())`) is available and `all=1`
  obviously can never qualify (no pinned start) — but the feed body's `<subtitle>` embeds
  `info.description`, read live from the repository's `[cgit]`/`[axgit]` config. That's mutable
  state riding on an otherwise sha-addressed resource, the same problem #45 solved for the commit
  page's git notes. The #45-shaped fix — `&& info.description.is_none()` — would work mechanically,
  but it only ever fires for undescribed repositories, and a sha-pinned Atom feed can never gain a
  new entry, so there's no reader that would meaningfully subscribe to one. Not worth a third
  variant of the immutability predicate for that trade. `get_feed` still returns `Ok((false, ...))`
  unconditionally.
- **`handlers::parse_limit` gained a `default: usize` parameter** (was hardcoded to `DEFAULT_LIMIT`
  = 50) so the feed's endpoint could reuse it with its own default of 20 instead of duplicating the
  "1–100, never clamped, `invalid_param` on failure" parsing logic. The three existing call sites
  (`commits`, `search`, `stats`) now pass `DEFAULT_LIMIT` explicitly.
- `docs/API.md`/`docs/openapi.json`/`web/src/lib/api/types.ts` updated (`GET /feed.atom` gained
  `ref`/`path`/`all`/`limit`, plus `400 invalid_param` and `404 ref_not_found` responses). The web
  surfacing is a follow-up commit (#62).

## #62 `/{repo}/log` and the repository summary page surface the feed parameters

The web half of #61 — API-then-page, same order #46/#47 and #59/#60 followed.

- **`CommitLog.tsx`'s action row, not the path-filter banner.** The existing "Filtered by path …"
  banner only renders when `resolvedPath` is set, so a `?ref=dev` log with no path filter would get
  no feed link at all if the link lived there. The row above it ("Expand messages · Show changes")
  already renders whenever there are commits to show and is already "options for the current
  view" — an `Atom feed` link carrying the log's current `ref`/`path` fits there as a third
  `·`-separated item. Deliberately forwards only `ref`/`path`: `msg`/`stat`/`follow`/`cursor` have
  no feed analogue, and the log page's page size is a different concept from the feed's item count,
  so `limit` isn't forwarded either.
- **`RepoSummary.tsx` gains a second `All refs` link** (`feedUrl(name, { all: 1 })`) beside the
  existing plain "Atom" link, inside the same `summary.head !== null` guard. Without it there is no
  UI path to `all=1` at all — "every branch and tag" is a repository-level concept, not something
  tied to a particular log view, so the summary page is where it belongs.
- `feedUrl(name, params)` gained an optional `FeedParams` argument; called with no arguments it
  produces the exact same URL as before (`buildQuery` already drops unset/empty values), so
  `RepoSummary.tsx`'s existing plain feed link and its test assertion needed no change — that's the
  no-regression proof for the signature change.
- No route change, no API contract change — `CommitLog.tsx`, `RepoSummary.tsx`, and
  `lib/api/repos.ts::feedUrl`/`FeedParams` only.

## #63 `<head>` Atom/`vcs-git` discovery on repository pages

Closes the last open "Feed and discovery" cgit-parity gap. cgit emits `<link rel="alternate"
type="application/atom+xml">` and `<link rel="vcs-git">` in every repository page's `<head>`; axgit
emitted neither — the feed/clone-URL links #61/#62 built out only exist as visible anchors inside
`RepoSummary.tsx`/`CommitLog.tsx`, both `client:only="react"` islands invisible to a feed reader or
any other tool that doesn't run JS, which is exactly who autodiscovery is for.

- **Server-side injection, not a client fill-in.** `/{repo}/*` pages are prerendered once under the
  `__repo__` placeholder param (#17), so the shell HTML has no per-repo href at build time — the
  same problem `window.__axgit.fillRepoShell` (`Layout.astro`) already solves for the heading/tab
  links/title. Filling the `<link>`s in the same way was considered and rejected: it would be
  invisible to exactly the non-JS consumers autodiscovery exists for, and `rel="vcs-git"` needs
  `clone_url_base`, which is api-side config the web build never sees. `api/src/shell.rs::serve_shell`
  already reads the shell file and knows the request path per request, so it injects the links
  itself, byte-wise, right before `</head>` — matching this file's and `feed.rs`'s "hand-build small
  fixed documents" stance (#12) rather than pulling in an HTML parser for three `<link>`s.
- **The segment is used raw, never decoded.** `repo_segment_for` returns the first path segment
  exactly as the browser sent it — still percent-encoded — and that's reused directly to build both
  `/api/v1/repos/{segment}/feed.atom` and `{clone_url_base}/{segment}.git`. Both want it
  percent-encoded exactly that way, so this skips the decode/re-encode `feed.rs::encode_segment`
  exists for. It's still run through the same `escape::xml_escape` (moved out of `feed.rs` into a
  new shared module, since both this file and the feed now need it) before interpolation, since an
  attribute value needs quote-escaping even though the segment shouldn't ever contain one in
  practice.
- **No repository-existence check.** The shell already answers `200` for a repository that doesn't
  exist (the island renders "Repository not found" client-side); adding a filesystem/config read to
  the static-serving path just to make one inert `<link>` disappear for that case isn't worth the
  I/O on every request.
- **Titles are fixed strings** ("Recent commits" / "Recent commits (all refs)" / "Git repository"),
  not the repository name — the name is only available percent-encoded here, and decoding it back
  to a display string would need machinery this serve path otherwise has no reason to carry.
- **`rel="vcs-git"` is omitted when `clone_url_base` is unset**, the same `null` rule
  `handlers/repos.rs::get_repo`'s `clone_url` field already follows, and built the same way:
  `format!("{}/{segment}.git", base.trim_end_matches('/'))`.
- **`?ref=` is deliberately not reflected** into the feed link (cgit's `h=`) — doing so would mean
  parsing and re-encoding a query parameter inside the static-serving path for a link most readers
  will use unparameterized anyway. Left as a possible follow-up if a real need shows up.
- **Verified against Astro's own source, not just its docs, that `<ClientRouter />` navigation
  doesn't need any extra wiring.** `swapHeadElements`
  (`node_modules/astro/dist/transitions/swap-functions.js`) removes every non-`transition:persist`ed
  child of the outgoing `<head>` and appends the incoming document's — so navigating between two
  repositories (or from `/` into one) always ends up with the new page's `<link>`s, no stale ones
  left behind.
- **`astro dev` (and so the Playwright e2e suite) never runs this.** `web/astro.config.mjs`'s
  `shellFallback` vite middleware only rewrites the incoming request's URL onto the matching page
  shell — it never touches the response body, so there's no dev-time equivalent of the injection to
  add. Full dev/production parity isn't achievable here regardless: `clone_url_base` is api-side
  config that Astro's dev server has no access to. Coverage is Rust-side only
  (`api/tests/static_shell_test.rs`), plus `Layout.astro` carries a comment recording why the
  feature doesn't appear anywhere in the web source. Documented as a known, permanent limitation
  rather than a gap to close later.
- No `docs/API.md`/`docs/openapi.json`/`web/src/lib/api/types.ts` change — no endpoint, no schema,
  no route shape changed; API.md doesn't cover static shell serving. Changed files:
  `api/src/shell.rs` (the feature), new `api/src/escape.rs` (shared `xml_escape`, moved out of
  `feed.rs`), `api/src/routes.rs` (threads `clone_url_base` through to the shell fallback),
  `api/tests/static_shell_test.rs`, `web/src/layouts/Layout.astro` (comment only).

## #64 Repository index sort (`?sort=`, `AXGIT_REPOSITORY_SORT`)

Closes the "Column sorting" cgit-parity gap under "Repository index" (docs/ROADMAP.md) — the index
was fixed to name order plus #25's client-side `?q=` filter, with no way to reproduce cgit's
`s=name|desc|owner|idle|section` or `repository-sort=age|name`. First of two commits — this one is
the API; the web page's sortable headers are a follow-up (#65).

- **`?sort=`, never cgit's `s=`.** #61's `?ref=`-not-`h=` precedent applies again: axgit's own query
  vocabulary, not a straight port of cgit's spellings.
- **A single param carries both column and direction**: `name`/`desc`/`owner`/`idle`/`section`,
  optionally `-`-prefixed to flip. `idle` is the one key whose *un-prefixed* form is descending
  (most recently active first, matching cgit's own `idle` sort); every other key's un-prefixed form
  is ascending. A leading `-` always flips a key's own default rather than meaning "always
  descending" — so `-idle` is ascending (oldest first), not a no-op. `RepoOrder::parse`/`Display`
  round-trip this exactly, which is what lets `ReposResponse.sort` echo the request's own spelling
  back unchanged.
- **`None` always sorts last, regardless of direction.** A repository with no `owner` shouldn't
  jump to the top under `-owner` just because reversing usually means "last things first" — reverse
  only flips the comparison between two *present* values (`repo/sort.rs::compare_opt_str`/
  `compare_idle`), never the `Some`-vs-`None` branches. Ties (including two repositories both
  missing the sorted field) break by `name` ascending, always, so the order is fully deterministic
  regardless of the snapshot's incoming order.
- **`idle` compares parsed timestamps, not the formatted string.** `meta::format_rfc3339` preserves
  each commit's/agefile's own UTC offset rather than normalizing to UTC, so two repositories
  recorded under different offsets would sort wrong by string order (`"+0900"` > `"-0500"`
  lexicographically, even when the `-0500` instant is later). `repo/sort.rs::compare_idle`
  re-parses each `last_modified` into a `jiff::Timestamp` before comparing.
- **Sorting moved out of `scan.rs` into the handler.** `scan_repos` used to sort by name as its
  last step; that made every possible order except name-ascending require a second full scan.
  `scan_repos` now returns directory-read order, and `list_repos` sorts a clone of the shared
  `ScanCache` snapshot per request — cheap at this scale, and it's the same clone
  `handlers/repos.rs` already made to serialize the response.
- **Server default via `AXGIT_REPOSITORY_SORT`** (`--repository-sort`, default `name`), parsed with
  the same `RepoOrder::parse`/`400 invalid_param` rule as the query param. A request's `?sort=`
  overrides it; when absent, `ReposResponse.sort` reports the configured default so a client never
  has to duplicate the server's own default logic to know what it received.
- **No response-cache work.** The list is served from `ScanCache` (single-value TTL, no key) with a
  body-hash `ETag` (#6's carve-out), not the moka response cache — a different `sort` produces a
  different body and therefore a different `ETag` automatically, with no cache-key change needed.
- `docs/API.md`/`docs/openapi.json`/`web/src/lib/api/types.ts` updated (`GET /api/v1/repos` gained
  `?sort=` and a `sort` response field, plus `400 invalid_param`). The web page surfacing sortable
  headers is a follow-up commit (#65).

## #65 Sortable repository-index column headers

Web-side follow-up to #64: `RepoList.tsx`'s Name/Description/Owner/Last activity headers become
clickable, closing the "Column sorting" gap end to end.

- **Client-side re-sort, not a refetch.** `GET /api/v1/repos` already returns every repository in
  one response (#25's same premise), so clicking a header re-sorts the array already in memory —
  `web/src/lib/repo-sort.ts`, a line-for-line mirror of `repo/sort.rs`'s rules (nulls-last
  regardless of direction, `idle`'s reversed default, `name`-ascending tiebreak, comparing parsed
  instants rather than the formatted `last_modified` string). A client-side sort and a server-side
  one of the same order never disagree.
- **`?sort=` is optional client state layered on top of the server's own order**, not a value the
  client must always carry. `RepoList` seeds `sortParam` from `?sort=` exactly once
  (`paramFromSearch`, same idiom as `?q=`); while it's unset, the rendered order — and which header
  shows `aria-sort` — is whatever the API actually returned (`ReposResponse.sort`, echoed back from
  #64), not a client-recomputed guess of the server default. This is what makes a server operator's
  `AXGIT_REPOSITORY_SORT` visible in the UI without the web build knowing it exists. An unparseable
  `sortParam` (a stale or hand-edited deep link) falls back the same way as if it were absent, same
  spirit as `filterRepos`'s empty-query no-op.
- **Clicking a different column always lands on that column's own default direction**
  (`defaultOrder`), never inheriting the previous column's direction — clicking "Owner" after
  "-idle" gives ascending owner, not descending. Clicking the *active* column toggles direction.
  `?sort=` is written via `history.replaceState`, not `pushState` — same #25 rationale (a history
  entry per click would fight the back button, and `<ClientRouter />` only reads `location` on a
  navigation, never observing the in-between state).
- **`section` has no header button.** It's `groupBySection`'s heading, not a table column — a
  clickable "Section" header would sort rows within each already-section-grouped table, which does
  nothing visible. `?sort=section`/`AXGIT_REPOSITORY_SORT=section` still work (they reorder which
  section appears first, and each section's own row order), just not from a header click.
- **No `DropdownMenu`.** `ThemeMenu.tsx`'s own note explains why: importing base-ui's `Menu` pulls
  in a ~137 KB floating-ui chunk, kept off every page except the one that already needs it. Plain
  `<button>`s inside each `<TableHead>` avoid the import entirely.
- **Found and fixed a real accessibility gap while wiring this up**: `components/ui/table.tsx`'s
  `TableHead` rendered a bare `<th>` with no `scope`. Per HTML-AAM a bare `<th>` should still map to
  the `columnheader` role, but Chromium's actual implementation doesn't do so reliably without an
  explicit `scope` — verified empirically with a minimal repro (a plain `<table><thead><tr><th>`
  outside this codebase's styling). Every `TableHead` in this app is a column header inside a
  `<thead>` row, so `scope="col"` is now the component's default (still overridable via `props`).
  This is what makes `getByRole("columnheader", …)` — used by this commit's own tests — resolve at
  all; it was silently broken for every existing table (Stats, Diff, Refs) before this fix, not
  just the new sortable one.
- No API contract change — `web/src/lib/repo-sort.ts`, `RepoList.tsx`, and the `table.tsx` `scope`
  fix only.

## #66 `hide`/`ignore` repository flags

Closes the "`hide`/`ignore` repo flags" cgit-parity gap under "Repository index": cgit's
`repo.hide`/`repo.ignore` distinguish a repository that's absent from the index but still
fetchable by direct URL (`hide`) from one that's unreachable by any means (`ignore`). Every
repository under `AXGIT_REPO_ROOT` was previously both listed and reachable, with no way an
operator could change either independently — this fits directly alongside the existing
`[cgit]`/`[axgit]` config-section invariant (#5), no new config surface.

- **Two flags, two different choke points, on purpose.** `hide` is enforced only in `scan.rs` (via
  a new `meta::should_list`), which is the sole producer of the list `ScanCache` holds — so a
  hidden repository simply never enters the snapshot `GET /api/v1/repos` serializes, while
  `GET /repos/{name}`, tree/blob/log/etc., and clone all keep working unchanged for it. `ignore` is
  additionally enforced in `open::open_named` (a new `meta::is_ignored` check right after the open
  succeeds) — the one function every per-repo handler and Smart HTTP call to turn a `{repo}` name
  into a `Repository`, so an ignored repository 404s (`repo_not_found`) everywhere, not just off
  the index. `should_list` treats `ignore` as also implying "not listed" (`!hide && !ignore`), so a
  repository doesn't need both flags set to disappear from the list — only from direct access does
  the distinction matter.
- **A boolean config reader, `config_flag`, added alongside the existing string one
  (`config_value`)** — same `[axgit]`-wins-over-`[cgit]` precedence (#5), same two-section
  `find_map`, just `get_bool` instead of `get_string`. `git2::Config::get_bool` already accepts
  every spelling cgit's own boolean options do (`true`/`false`, `yes`/`no`, `on`/`off`, `1`/`0`),
  so no custom parsing was needed — unlike `handlers/mod.rs::parse_flag`, which exists specifically
  because *query* params need a `400 invalid_param` on a bad value; a config flag has no such
  channel and silently defaults to `false` instead, matching how a missing/unparseable
  `section`/`owner`/`desc` already resolves to `null` rather than an error.
- **No API contract change.** Neither flag is a response field — `hide`'s effect is "which
  repositories does `GET /api/v1/repos` enumerate," and `ignore`'s is "does `open_named` succeed at
  all," both index/reachability behaviour rather than new data. `RepoInfo`/`RepoSummary` are
  unchanged.
- `scripts/make-fixtures.sh` gained `hidden.git` (`cgit.hide`) — the one place the hide-but-
  reachable distinction is visible end to end against the real filesystem scan, since the api's own
  tests build per-test fixtures in a tempdir instead (`api/tests/repos_test.rs`'s new
  `setup_hide_ignore_fixtures`, covering both flags plus refs/clone reachability for `ignore`).

## #67 Per-repository `homepage`

Closes the `homepage` half of the "`homepage` (cgit gives it a dedicated nav tab), and a configured
`defbranch`" cgit-parity gap under "Repository index" — `defbranch` is a separate, behavioral
commit (#68), since it changes ref resolution rather than adding a field.

- **`RepoInfo`/`RepoSummary` both gain `homepage: Option<String>`**, read via `meta("homepage")` —
  the exact same `[axgit]`-wins-over-`[cgit]` `config_value` lookup `section`/`owner`/`desc` already
  use, no new precedence rule.
- **Only `http://`/`https://` values survive**; anything else — most importantly a `javascript:`
  URL — reads back as `null`, the same as if `homepage` were unset. This is the one config-derived
  field so far that lands directly in an `href` rather than being rendered as inert text
  (`section`/`owner`/`desc` are always text nodes), so a malicious value in `cgit.homepage` would be
  a stored XSS if passed through unchecked. Filtering at the read (`meta::is_http_url`) means the
  invariant holds for every consumer without each one having to re-validate.
- **Rejected rather than erroring at scan time.** A misconfigured `homepage` (missing scheme, a
  bare hostname, `mailto:`, …) degrades to `null` — cgit itself doesn't validate `homepage` either,
  and failing the whole repository's listing over one bad config value would be a worse failure
  mode than silently dropping one field.
- `docs/API.md`/`docs/openapi.json`/`web/src/lib/api/types.ts` updated (`RepoInfo` and
  `RepoSummary` both gain `homepage`). `scripts/make-fixtures.sh`'s `git-compose.git` gained
  `cgit.homepage` so a local run reaches the field. The web page rendering it is a follow-up
  commit (#69), same two-commit shape as #64/#65's sort work.

## #68 Honour the configured `defbranch`

Closes the last piece of "`homepage` (cgit gives it a dedicated nav tab), and a configured
`defbranch`" — behavioral, unlike #67's `homepage` field: axgit only ever derived the default
branch from HEAD, so every "no ref given" request (summary, commit log, tree/blob/raw/blame,
archive, diff, stats, search, feed) resolved against HEAD regardless of `defbranch`.

- **One choke point, not ten.** `resolve::resolve_commit` — already the single function every ref
  resolution in the api goes through — substitutes the configured `defbranch` for the *literal*
  `"HEAD"` string before `revparse_single`, gated on an exact `refname == "HEAD"` check so the
  config read only happens on that one input, never on an explicit branch/tag/sha. This is what
  makes `/tree/HEAD/...`, `/blob`, `/raw`, `/blame`, `/archive/HEAD.*`, and `/diff?from=HEAD`/
  `?to=HEAD` all honor `defbranch` for free — `diff.rs`'s and `files.rs`'s readme handler already
  built the literal string `"HEAD"` as their own "no ref given" default and fed it straight into
  `resolve_commit`/`resolve_ref_path`, so neither needed a single line changed. Only the four sites
  that called `repo.head()` directly, bypassing `resolve_commit` entirely — `commits.rs`,
  `stats.rs`, `search.rs`, `feed.rs` — were switched to a new `resolve::default_commit`, plus the
  repository summary's own `head` field (`handlers/repos.rs::get_repo`), which would otherwise show
  a different tip than every other tab.
- **A stale or misconfigured `defbranch` degrades to HEAD, silently.** `meta::configured_default_branch`
  checks `repo.find_branch(name, Local)` before returning the value — a renamed or deleted branch
  doesn't 404 the whole repository on every ref-less request, it just behaves as if `defbranch`
  were never set. Same reasoning as `homepage`'s scheme filter (#67): one bad config value degrades
  one field/behavior, never fails the request outright.
- **`RepoInfo.default_branch`/`RepoSummary.default_branch` now report the configured branch too**
  (`meta::default_branch` prefers `configured_default_branch` over HEAD's own shorthand) — no
  schema change, since the field already existed and was always `Option<String>`.
- **Left on HEAD, deliberately**: `meta::validator` (the cache freshness check — any ref movement,
  defbranch or not, should still be a change signal) and `last_modified`'s `head_authordate`
  fallback (a push to a non-HEAD `defbranch` still touches the agefile, so freshness tracking is
  unaffected either way).
- **No cache-key change.** `HEAD` and an explicit branch name remain distinct `params` strings in
  every `cached_response` call — resolving to the same commit under `defbranch` just means two
  cache entries with identical bodies, not a correctness problem. A `defbranch` edit itself is
  config-only and invisible to the HEAD/agefile validator, so it inherits the existing
  `AXGIT_CACHE_RESPONSE_TTL` staleness ceiling (docs/ARCHITECTURE.md's documented safety net for
  exactly this class of out-of-band change) — same as `homepage`'s caching story in #67.
- `docs/API.md` updated (the ref-parameter convention, `default_branch`, and the summary's `head`
  field) — no schema change, so no `docs/openapi.json`/`web/src/lib/api/types.ts` diff.
  `scripts/make-fixtures.sh`'s `dotfiles.git` gained a `legacy` branch (created before its second
  commit, so it diverges from `main`) plus `cgit.defbranch legacy`, so a local run can tell
  `defbranch` apart from HEAD by more than a config key existing. New
  `api/tests/defbranch_test.rs` covers the summary, the ref-less commit log, and the ref-less tree,
  plus the unset and nonexistent-branch fallback cases.

## #69 Link each repository's homepage

Web-side follow-up to #67: renders `RepoSummary.homepage`/`RepoInfo.homepage` as an actual link,
closing the "`homepage` (cgit gives it a dedicated nav tab)" cgit-parity gap under "Repository
index" — the `homepage` field has existed in the API since #67, but nothing on the web read it.

- **Not a nav tab, unlike cgit.** `RepoNav.astro`'s tab bar is static Astro, prerendered once under
  the `__repo__` placeholder param (#17); every tab's href is rewritten same-site by
  `window.__axgit.fillRepoShell` (`Layout.astro`), which only knows how to build `/{segment}/{sub}`
  — it has no notion of an arbitrary external URL, let alone one that may or may not exist per
  repository. A conditionally-present, external-URL tab doesn't fit that model without rebuilding
  it, so `homepage` becomes a link instead, in the two places that already show per-repository
  metadata: a new `MetaItem` row on `RepoSummary.tsx` (placed with `section`/`owner`, the other
  optional descriptive fields) and a third `IconLink` in `RepoList.tsx`'s per-row actions cell
  (alongside #48's Log/Tree quick links). Both are rendered only when `homepage` is non-null.
  Recorded here as a deliberate, permanent difference from cgit rather than a gap to close later.
- **`target="_blank"` + `rel="noopener noreferrer"`, and only for this link.** Both `MetaLink`
  (`RepoSummary.tsx`) and `IconLink` gained an `external` prop that adds these attributes — every
  other use of either component points back into axgit itself (refs, archive downloads, the feed,
  Log/Tree), so `external` defaults to unset/`false` and every existing call site is unchanged.
  `homepage` is the first link in the app to genuinely leave the site.
- `web/tests/fixtures/repos.json` gained `homepage` (`git-compose` set, the other three `null`) —
  shared by the `RepoList` unit test, `repo-filter` test, and the e2e spec, so a single fixture
  change exercises both the present and absent cases everywhere it's used.
- No API contract change — `RepoSummary.tsx`, `RepoList.tsx`, `IconLink.tsx`, and test fixtures
  only.

## #70 Site title, description, and readme

Closes "Site-level readme / title / description" — the last open "Repository index" cgit-parity
item — with the API half: three new `Config` fields, a new `GET /api/v1/site`, and server-side
`<head>` injection so the site's own identity isn't hardcoded to "Axgit" everywhere. The web page
actually rendering these is a follow-up commit (#71), same two-commit shape as #64/#65 and #67/#69.

- **A new, non-repo-scoped endpoint, not fields tacked onto `GET /api/v1/repos`.** Title/
  description/readme describe the *deployment*, not any repository, and the readme in particular
  needs its own response shape (`format`/`content`, mirroring `repo/readme.rs::ReadmeInfo` without
  its repo-specific `path`) — folding them into the repo list would conflate two different
  resources the way `RepoInfo`/`RepoSummary` were deliberately kept apart from the start (#1-era
  decision, still honored by #64-#69). `title` is always present (falls back to `"Axgit"`);
  `description`/`readme` are `null` when unset, the standard rule (docs/API.md:18).
- **The readme is read from the filesystem at request time, with no traversal check.**
  `AXGIT_ROOT_README` is operator configuration passed on the command line/environment, not user
  input reachable from any request parameter — the same trust boundary `AXGIT_REPO_ROOT` itself
  already sits on. Capped at 512 KiB (matching the per-repository blob/readme limit) so a
  misconfigured path pointing at a huge file can't blow up response size; over the cap, missing, or
  non-UTF-8 all degrade to `readme: null` rather than failing the whole response — `title`/
  `description` are unaffected either way, the same "one bad config value degrades one field"
  stance #66/#67/#68 already established for `hide`/`homepage`/`defbranch`.
- **Format is guessed from the configured path's extension**
  (`.md`/`.markdown`→markdown, `.rst`→rst, else plain), reusing `repo/readme.rs::ReadmeFormat`
  itself rather than duplicating the three-value enum. This is a *different* guess from
  `repo/readme.rs::CANDIDATES`'s name-based one (`README.md` vs. `README.rst` vs. …) — an
  operator-chosen path has no fixed candidate list to match against, only an extension to read.
- **Not cached at all** — no `ScanCache` entry, no moka response-cache entry — same "not tied to a
  repository" treatment the repo list gets (#6's carve-out), just with nothing to snapshot in the
  first place: config is already in memory and the readme read is one stat plus one read. A
  body-hash `ETag` still gives clients a 304 path.
- **`<head>` injection generalized, not duplicated.** #63's `inject_repo_head_links` became
  `inject_before_head_close(body, extra)`, a pure byte-splice taking pre-built markup instead of
  building repo `<link>`s itself; a new `site_head_meta` builds `<meta name="axgit:site-title">`/
  `<meta name="axgit:site-desc">` (omitted when unset), and `serve_shell` concatenates both extras
  before a single splice. **Custom `axgit:` meta names, not overwriting the real `<title>`/
  `<meta name="description">` directly** — the shells are prerendered once per route *shape*, so
  the real elements' content is already baked in at build time; overwriting them server-side would
  work for `<head>` itself, but the header **brand text** is in `<body>`, which needs client-side
  filling regardless (exactly `fillRepoShell`'s existing job for the repository name) — so the
  custom metas exist to smuggle config into the browser for one client script (`fillSiteChrome`,
  #71) to apply consistently to both places, rather than having two different mechanisms (one
  server-side for `<head>`, one client-side for `<body>`) disagree or race.
- **Injected into *every* shell — index, repo pages, and 404 — unlike #63's repo-only `<link>`s.**
  A site title/description is deployment-wide, not per-repository.
- **Found and fixed a real bug while wiring this up**: `GET /` never reached `serve_shell` at all.
  `ServeDir`'s default `append_index_html_on_directories(true)` serves `static_dir/index.html`
  directly for a bare `/` request — the only route shape in this build that corresponds to a real
  on-disk directory — bypassing the `.fallback(shell)` closure (and so #63's own injection logic,
  had the index page ever needed it) entirely. Invisible before now because #63 only injects into
  repo shells, and the index page never carried per-repo links to miss. Fixed by
  `.append_index_html_on_directories(false)` on the `ServeDir` in `routes.rs`; every other route
  shape was already unaffected, since none of them correspond to a real directory on disk (only
  `__repo__/` does, and nothing requests that literal path).
- `docs/API.md`/`docs/openapi.json`/`web/src/lib/api/types.ts` updated (`GET /api/v1/site`, new
  `site` tag). `README.md`/`docs/ARCHITECTURE.md`/`docs/compose.example.yaml` gained
  `AXGIT_ROOT_TITLE`/`AXGIT_ROOT_DESC`/`AXGIT_ROOT_README`. New `api/tests/site_test.rs`
  (defaults, configured values, readme format guessing, missing/oversized readme, ETag round-trip)
  and `static_shell_test.rs` cases for the injected metas across index/repo/404 shells and their
  coexistence with #63's repo `<link>`s.

## #71 Render the site title, description, and readme

Web-side follow-up to #70, and the close of the last open "Repository index" cgit-parity item.

- **A shared `ReadmeBody.tsx`, split out of `ReadmeView.tsx`.** `ReadmeView` owned three things
  that don't all apply to a site-level readme: fetching (`getReadme`), the "path" heading, and the
  format-driven render (`markdown` → lazy `ReadmeMarkdown` in a `Suspense`, else a `<pre>`). Only
  the third is shared with `SiteIntro.tsx`, which already has a `SiteReadme` in hand from its own
  `GET /api/v1/site` fetch and has no per-repo path to head with. Splitting keeps the lazy-loading
  boundary (react-markdown/remark-gfm/rehype-sanitize, ~135 KB) in exactly one place rather than
  duplicating the `Suspense`/`lazy` dance in `SiteIntro`, and `ReadmeView` itself is now three
  lines shorter with identical behavior (its own test suite is unchanged and still passes).
- **`ReadmeMarkdown`'s `repo` prop becomes optional.** Its relative-link/image rewriting
  (`resolveRepoPath` → `/{repo}/tree|blob/...` or `rawUrl(repo, ...)`) is meaningless for a
  site-level readme, which has no single owning repository — a link `./guide.md` in the root
  readme has no repository tree to resolve against. `repo === undefined` now short-circuits both
  `rewriteHref`/`rewriteSrc` to pass every URL through unchanged, so `SiteIntro`'s readme renders
  with its relative links exactly as written (resolved by the browser against the current page),
  rather than being rewritten into a nonsensical `/{repo}/...` href with an empty `repo`.
- **`SiteIntro` renders nothing at all when `title === "Axgit"` (the api's own default) and
  `description`/`readme` are both `null`** — an unconfigured deployment's index page is
  byte-identical to before this commit. This can't distinguish "nothing configured" from
  "`AXGIT_ROOT_TITLE` explicitly set to the literal string `Axgit`", but that's the same collapse
  `api/src/site.rs::effective_title` already makes server-side, not a new ambiguity introduced
  here. The header brand (`fillSiteChrome`, below) already shows the title regardless, so this
  component only adds visible content when there's something *beyond* that default to say.
- **`fillSiteChrome`, not a second injection mechanism.** Defined in `Layout.astro`'s existing
  `data-repo-shell-init` script, right next to `fillRepoShell`, sharing its `window.__axgit` guard
  and its `astro:after-swap` registration — the same reasoning #63/#70 already established: every
  shell carries this script, so the listener must be live before the first navigation regardless of
  page type. It reads the `axgit:site-title`/`axgit:site-desc` `<meta>`s #70 injects server-side
  and applies them to three places: the header brand (`[data-site-title]`, a plain-text swap,
  `transition:persist`ed so this only has to happen once per document, not per navigation), the
  real `<meta name="description">`'s `content` attribute (not persisted — reverts to the shell's
  build-time default on every swap, so this genuinely needs the listener to reapply it each time),
  and — **only on non-repository pages** (checked via the absence of `[data-repo-name]`, the same
  marker `fillRepoShell` itself keys off) — `document.title`. A repository page's title stays
  `fillRepoShell`'s own `name + suffix`; `RepoLayout.astro`'s `TITLE_SUFFIXES` are static strings
  baked at Astro build time (`" — Axgit"`, `" log — Axgit"`, …) with no runtime site-title
  interpolation, and rebuilding that as a runtime template was left out of this commit's scope —
  recorded here as a deliberate, revisitable limitation rather than a bug.
- **A one-time invocation, not just the listener.** The listener alone only fires *after* a client
  navigation; the *first* load of any page (the common case — most sessions start on a hard
  navigation, and this codebase runs no server-rendered soft-hydration) needs `fillSiteChrome()`
  called once directly. `fillRepoShell` gets this from `RepoLayout.astro`'s own body-level inline
  script, but that only exists on repository pages — `fillSiteChrome` needs it on *every* page
  (`/`, `/404`, and every repository page all show the header brand). Added as a small
  `<script is:inline>` in `Layout.astro`'s own body, right after `</header>` — by the time an inline
  script this far down executes, the preceding `<header>` (and so `[data-site-title]`) is already
  in the DOM, the same "runs as soon as what it needs exists" property `RepoLayout.astro`'s own
  call already relies on. Calling it here also means Astro's swap machinery re-runs it (along with
  every other un-persisted body script) on every navigation too — redundant with the `astro:after-swap`
  listener, but harmless: the function is idempotent.
- **e2e coverage stops at the client half, like #63's.** `astro dev` (what the Playwright
  `webServer` runs) never executes `api/src/shell.rs`'s server-side `<meta>` injection — the same
  documented gap #63 left. `web/e2e/site.spec.ts` covers `SiteIntro`'s rendering (mocking
  `GET /api/v1/site` directly, which needs no server injection at all) and, for `fillSiteChrome`,
  injects the `axgit:site-*` metas by hand via `page.evaluate` and calls the function directly —
  exercising the real client logic in a real browser without needing the production Rust binary.
- No API contract change — `web/src/components/repo/ReadmeBody.tsx` (new),
  `ReadmeMarkdown.tsx`/`ReadmeView.tsx` (refactored), `web/src/components/SiteIntro.tsx` (new),
  `web/src/lib/api/site.ts` (new), `web/src/lib/api/schemas.ts` (`SiteInfo`/`SiteReadme` exports),
  `web/src/layouts/Layout.astro`, and `web/src/pages/index.astro` only.

## #72 Submodule (`module-link`) links

Closes the "Submodule (gitlink) links" cgit-parity gap under "Tree and blob" (`module-link` /
`repo.module-link.<path>`) — after this, single-child directory collapsing is the only cgit-parity
item left open. `TreeEntryInfo` already carried everything needed except the link destination
itself: a gitlink's `sha` (the submodule commit, in the *other* repository) and `name`.

- **Two sources, config first, `.gitmodules` as a fallback — an axgit extension cgit itself doesn't
  have.** cgit only ever reads `cgitrc`; axgit additionally reads `.gitmodules` from the resolved
  commit when no config template applies, using the mapped `url` verbatim if it's `http://`/
  `https://`. This exists because a config template requires an operator to go add one, one path at
  a time, while `.gitmodules` is content the repository already carries — a submodule that's a
  public GitHub/GitLab project gets a working link with zero configuration.
- **Four config keys, but no new precedence machinery.** `axgit.<path>.module-link` →
  `cgit.<path>.module-link` → `axgit.module-link` → `cgit.module-link`, where `<path>` is the
  gitlink's full repo-relative path. This is exactly `meta::config_value`'s existing
  `[axgit]`-wins-over-`[cgit]` lookup (#5), called once per level
  (`meta::module_link_template(cfg, path)`), so path specificity is checked *before* section
  precedence — a `[cgit "<path>"]` entry beats a repo-wide `[axgit]` one. `git config` itself
  accepts a `/`- and `.`-bearing subsection like `axgit.vendor/lib.js.module-link` without issue
  (verified against the real `git config` binary before committing to this key shape) — libgit2
  splits a key on its *first* and *last* dot, so a dotted path round-trips correctly, and the
  subsection is case-sensitive, matching how a tree path should compare.
- **The first applicable source wins, even when its value turns out to be unusable — it never falls
  through.** If any of the four config keys is set at all, that's the answer (`Some(link)`, possibly
  `None` after validation), full stop; `.gitmodules` is only consulted when *no* config key is set.
  This buys one thing for free: an operator sets `axgit.<path>.module-link` to the empty string to
  explicitly suppress a repo-wide template (or a `.gitmodules` mapping) for one path, without that
  degrading into "well, try the next source instead". A typo'd or since-broken template also stays
  silently linkless rather than surprising the operator with a `.gitmodules` URL nobody asked for.
- **Template grammar: two `%s` placeholders (path, then sha), `%%` for a literal `%`, everything else
  involving `%` makes the whole template unusable.** A third `%s`, an unrecognized specifier
  (`%d`, `%1$s`, …), or a trailing lone `%` all return `None` rather than substituting an empty
  string or guessing — "no link" is diagnosable in the tree view (the row just isn't a link);
  "silently wrong link" is not. There's no cgit behavior to match here either: unlike C's `printf`,
  which would read whatever happens to be on the stack for a third argument, there is nothing
  meaningful to read.
- **Substituted values are inserted verbatim, not percent-encoded — matching cgit's own
  `html_attrf`-style substitution closely enough that an existing cgitrc value can usually be pasted
  in unchanged.** This is safe specifically because the *scheme* comes from the template, not from
  the substituted path, and the href guard below runs on the **expanded result**, not the template
  string — a gitlink literally named `javascript:alert(1)` under a bare `%s` template is still
  rejected, because by the time the guard runs, `%s` has already become that string and gets
  filtered like any other value.
- **Two distinct href guards, not one.** `meta::is_http_url` (promoted from private to
  `pub(crate)`, now documented as the shared "does this belong in an `href`" primitive) is kept as
  the strict `http(s)`-only check for `.gitmodules`, since a `.gitmodules` `url` is routinely a local
  filesystem path (`/srv/git/dep.git`) or an SSH remote (`git@host:owner/repo.git`) — neither
  belongs in a same-site href. A separate, deliberately looser `submodule::is_link_href` covers the
  config-template result: it additionally accepts a single leading `/` (root-relative), because
  cgit's own documented `module-link` example is exactly that shape
  (`/git/%s/commit/?id=%s`) — the natural form for an operator whose forge sits behind the same
  reverse proxy as axgit.
  - **A leading `/` immediately followed by another `/` or a `\` is rejected**, not just bare `//`:
    both `//evil.com/x` and `/\evil.com/x` are folded into "protocol-relative" by browser URL
    parsers (a backslash is normalized to a forward slash in the URL's "special authority slashes"
    state), so either would navigate off-site despite starting with a single `/`.
  - **A relative template is rejected outright** — cgit's *other* documented example
    (`./?repo=%s&page=commit&id=%s`) is exactly this shape, and it's the one cgitrc value that
    genuinely cannot be pasted into axgit unchanged: a relative href resolves against whatever tree
    path the browser happens to be showing (`/{repo}/tree/a/b/c`), so the same config value would
    point somewhere different depending on how deep into the tree the gitlink is nested — not
    something one repo-wide (or even per-path) string can mean consistently.
- **`.gitmodules` is hand-parsed, not read via git2/libgit2's own submodule API.** Verified before
  writing the parser: git2 0.20.4's `Config` has no in-memory/buffer constructor (`open`/`add_file`
  are path-based only), and libgit2's own `gitmodules_snapshot` (`submodule.c`) returns
  `GIT_ENOTFOUND` whenever `git_repository_workdir(repo) == NULL` — which is unconditionally true for
  every bare repository axgit ever opens. `Repository::submodules()` is therefore a dead end here,
  and axgit reads the blob directly. The parser (`submodule::parse_gitmodules`) is pure and
  line-oriented: `[submodule "name"]` stanzas are matched case-insensitively, `path`/`url` keys are
  matched case-insensitively with last-within-a-stanza-wins (git's own semantics), and a `path` seen
  in an earlier stanza wins over a later duplicate. Keyed on `path`, deliberately **not** the stanza
  name — git allows the two to differ, and only `path` addresses anything in the tree. Capped at 64
  KiB before loading (checked via an object-header stat, the same `SYMLINK_TARGET_LIMIT` pattern
  `tree.rs` already uses for symlink targets) — comfortably past any real `.gitmodules`, and an
  oversized file yields no links at all rather than a parse of truncated (and therefore wrong) input.
  Deliberately unhandled, and documented as such in the module doc comment rather than silently
  mishandled: line continuations, multi-line quoted values, `[include]`/`includeIf`, and relative
  `url` values (`../dep.git`, meaningful only relative to the superproject's own remote, which a
  bare repository doesn't have).
- **`entries_of` (shared by `GET /tree` and `GET /objects/{oid}`'s tree case) is left untouched;
  `list_tree` calls a new `submodule::fill_module_links` afterward instead.** The alternative —
  threading a resolution context through `entries_of` — was rejected because the one optimization
  that matters (skip config/`.gitmodules` entirely when a listing has no gitlink at all) can only be
  expressed *after* the entries already exist, and because `GET /objects/{oid}` has no commit/path
  context to resolve a template against in the first place; passing it `None` there would just be a
  sentinel restating what the response's own `module_link: null` already says. `fill_module_links`
  is infallible (`()`, not `Result`) — every failure degrades this one field, the same posture
  `homepage` (#67) and `defbranch` (#68) already established — and lazily loads `.gitmodules` at
  most once per listing, only when some gitlink's config lookup actually misses.
- **Frontend: `entry.module_link ?? undefined` replaces the flat `undefined` `TreeView.tsx`'s
  `entryHref` previously returned for every `commit`-typed row**, and the by-oid `ObjectView.tsx`
  keeps its existing unlinked rendering unchanged — under this design `module_link` is always `null`
  there, so no behavior needed to change, only its explaining comment.
  - **`rel="noopener noreferrer"` and `data-astro-reload`, deliberately no `target="_blank"`** — and
    deliberately *not* a reuse of #69's `homepage`-style `external` prop. `module-link`'s destination
    may be same-site (the `/git/%s/…` shape) or genuinely off-site, and a per-row sniff would make
    two visually identical submodule rows behave differently depending on what the operator happened
    to configure; treating "continue browsing the source elsewhere" as a plain navigation rather than
    a side trip (`homepage`'s framing) avoids that split. `rel="noopener noreferrer"` without
    `target` is inert for `noopener` (no new browsing context is ever created) but still suppresses
    the `Referer` header, which costs nothing and covers the off-site case. **`data-astro-reload` is
    the load-bearing part**: verified directly against `<ClientRouter />`'s own source
    (`astro/components/ClientRouter.astro`) that its click handler checks
    `el.dataset.astroReload !== undefined` before intercepting a same-origin click for a client-side
    swap — without it, a same-site `/git/…` destination behind the same reverse proxy would get its
    HTML response spliced into the axgit shell instead of loading as its own page, since same-origin
    is the only condition `<ClientRouter />` checks (it has no way to know the destination is a
    different application). The existing `rawUrl`/archive/raw links in this codebase escape this
    only because the API responses they point at aren't `text/html`.
- **Caching splits along the same line the two sources do.** The config half is invisible to the
  HEAD/agefile validator (docs/ARCHITECTURE.md#caching) — same caveat #67 (`homepage`) and #68
  (`defbranch`) already documented for config-only changes — so a `module-link` edit with no
  accompanying push inherits the `AXGIT_CACHE_RESPONSE_TTL` ceiling rather than invalidating
  immediately. The `.gitmodules` half, by contrast, is repository content: it moves with HEAD like
  any other file and invalidates the normal way.
- `docs/API.md`/`docs/openapi.json`/`web/src/lib/api/types.ts` updated (`TreeEntryInfo.module_link`,
  plus `GET /objects/{oid}`'s tree entry shape). New `api/src/repo/submodule.rs` (`fill_module_links`,
  `expand_template`, `is_link_href`, `read_gitmodules`, `parse_gitmodules`, each with their own unit
  tests), `meta.rs` gained `module_link_template` (plus its own precedence tests) and promoted
  `is_http_url` to `pub(crate)`. New `api/tests/module_link_test.rs` exercises the full precedence
  chain end to end through the router (config vs. `.gitmodules`, per-path vs. repo-wide, the nested-
  path substitution case, the by-oid tree's `null`). `files_test.rs`'s existing tree assertions
  gained `"module_link": null`, pinning "no config, no `.gitmodules` → no link" alongside the shape
  check. `TreeView.test.tsx`/`ObjectView.test.tsx` fixtures updated to match, plus new `TreeView`
  cases for a linked and a root-relative submodule row.

## #73 Validate `--chart-2..5`

Closes the roadmap candidate #29 and #33/#34 each deferred: `--chart-2..5` had never been run
through the `dataviz` skill's `validate_palette.js`, because nothing consumed them (the stats chart
is single-series; the commit graph and ref badges both deliberately encode identity by shape/icon
instead, exactly per the method's own "identity is never colour-alone" rule — not as a stand-in for
missing colour). Converted to hex, the shadcn-generated values turned out to be a *broken* palette,
not merely an unverified one, so this is a real fix rather than a formality.

- **Measured, not assumed.** The four slots are Tailwind's teal ramp (`#00bba7`/`#009689`/
  `#00786f`/`#005f5a`) — one hue (182–188°) at four lightness steps, byte-identical in light and
  dark (they never switched with the theme at all). Run against `--background` (light `#ffffff`,
  dark `#090b0c` — `StatsChart` paints directly on the page, not inside a `Card`), the validator
  hard-fails both modes: chroma floor (`#00786f` 0.09, `#005f5a` 0.076, both below the 0.10 floor —
  reading as gray), the normal-vision floor (worst adjacent ΔE 7.9, floor 15), and in dark mode the
  lightness band too (`#00bba7` L 0.71 above the 0.48–0.67 band, `#005f5a` L 0.438 below it). Slot 1
  was re-checked alone and still passes in both modes, confirming #29's fix held.
- **Replacement: four hues from the skill's reference categorical palette
  (`references/palette.md`), in a derived order, slot 1 unchanged.** All 1680 orderings of 4 hues
  from the reference eight (excluding `green`, whose family slot 1 already occupies) were fed
  through the validator for both modes against the real surfaces; 244 passed every hard gate. The
  order kept is the one maximizing the worst adjacent CVD ΔE, needing no contrast relief in either
  mode: slot 2 blue, slot 3 orange, slot 4 violet, slot 5 red. Light: `#2a78d6`/`#eb6834`/`#4a3aa7`/
  `#e34948`; dark: `#3987e5`/`#d95926`/`#9085e9`/`#e66767` — every hex verbatim from the reference
  file (check 6: documented palette only). Result: **ALL CHECKS PASS** both modes — light worst
  adjacent CVD ΔE 19.8 (deutan) / normal-vision 20.9; dark worst adjacent CVD ΔE 18.8 (deutan) /
  normal-vision 20.0; all 5 slots ≥ 3:1 against `--background` in both modes, and dark also clears
  3:1 against `--card` (`#161b1d`, the tooltip/popover surface). Unlike the old values, light and
  dark are stepped separately now.
- **The hexes were converted back to `oklch()` with a round-trip check**, since `global.css` is
  written in that form: each string was verified to convert back to its documented hex exactly
  before landing (`--chart-2` light needed a third hue decimal — `255.53`, not `255.5` — to avoid a
  one-bit drift to `#2978d6`).
- **No new consumer.** `StatsChart` stays single-series; `CommitGraph` (#33) and `RefBadges` (#34)
  keep their shape/icon encodings — both already cite the dataviz method's identity-is-never-
  colour-alone rule directly rather than "the palette isn't ready yet", so this fix doesn't change
  either decision, it just removes the stale justification `RefBadges.tsx`'s comment gave.
- **Correcting #29's own prose while re-running its work**: it describes light mode's original
  `--chart-1` (OKLCH L 0.855) as failing on "~1.4:1 contrast". The validator's contrast check is a
  non-blocking `relief` WARN (visible labels or a table view satisfy it), not a hard FAIL — the
  actual hard FAIL in light mode was the same one as dark, the lightness band (0.855 sits above the
  light band's 0.77 ceiling). The fix #29 shipped was correct either way; only the stated reason for
  light mode was imprecise. Recorded here rather than editing #29, which stays as written.
- `web/src/styles/global.css` (`:root`/`.dark` chart blocks, comments rewritten) and
  `web/src/components/repo/RefBadges.tsx` (comment only) are the only files touched. No API
  contract change, no new route, no test change — nothing renders a second series yet, so there is
  nothing to newly assert on.

## #74 Single-binary build: `embed-web` Cargo feature

Closes the embedding half of the roadmap's single-binary, non-container deploy candidate. Until
now the only way to serve the frontend was `AXGIT_STATIC_DIR` pointing at a `web/dist` directory
shipped alongside the binary — fine for the container (`COPY --from=web /app/web/dist /app/dist`),
but a bare-metal/systemd install would have to ship and keep in sync two artifacts whose versions
must match exactly. `git` exec (archive/upload-pack) stays a runtime dependency regardless of this
feature — this closes only the frontend half.

- **`rust-embed`, behind an opt-in `embed-web` feature (`api/Cargo.toml`)**, not `include_dir`:
  `rust-embed`'s `EmbeddedFile::metadata()` gives a sha256 hash and a `mime_guess`-backed content
  type for free per file, both of which the serving path needs anyway — `include_dir` would need
  those computed by hand. The feature is opt-in (`embed-web = ["dep:rust-embed"]`) so the default
  build carries no compile-time dependency on `web/dist` existing at all; the Dockerfile is
  untouched and keeps using `AXGIT_STATIC_DIR`. `debug-embed` is enabled unconditionally so
  `cargo test --features embed-web` exercises the same embedded-lookup code path release builds
  use (without it, `rust-embed` reads from disk in debug, a different path); `mime-guess` reuses
  the same `mime_guess` crate `repo/blob.rs` already depends on, so content types agree between the
  embedded and `AXGIT_STATIC_DIR` serving modes; `deterministic-timestamps` drops the embedded
  per-file mtimes (never read by this crate) so the compiled binary doesn't vary with checkout
  time, matching the reproducibility stance the Dockerfile's pinned base images already take (#22).
  `Cargo.lock` gains `rust-embed` even though the container build never enables the feature —
  harmless, `cargo fetch --locked` just fetches an unused optional dependency.
- **`api/src/assets.rs`'s `Assets` enum (`Dir(PathBuf)` / `Embedded`) is the single place that knows
  where the build is read from**; `Assets::resolve` prefers a configured `AXGIT_STATIC_DIR` and
  only falls back to the embedded copy when unset — an operator can still override a baked-in build
  without rebuilding. `api/src/shell.rs::serve_shell`/`serve_shell_or_redirect` take `Assets`
  instead of a bare `PathBuf`, reading through `Assets::read`, so the page-shell logic
  (`shell_for`'s route-shape mapping, `<head>` injection, cgit-compat redirects) is byte-for-byte
  shared between both modes — nothing in `shell.rs` besides the two function signatures changed.
  `api/src/routes.rs` picks the outer fallback per mode: the directory arm is unchanged
  (`ServeDir::new(dir).append_index_html_on_directories(false).fallback(shell)`); the embedded arm
  is a single handler that tries `assets::serve_embedded_file` first and falls through to
  `shell::serve_shell_or_redirect` on `None`, mirroring `ServeDir`'s own fallback ordering without
  touching the filesystem.
- **`serve_embedded_file` mirrors `ServeDir`'s behaviour deliberately, not incidentally**: no
  `Cache-Control` (content-hashed `_astro/*` assets could safely go immutable, but that's a
  mode-independent improvement, left as a ROADMAP candidate rather than bundled in here), a strong
  `ETag` (sha256 of the file, reusing `handlers/mod.rs::if_none_match`/`not_modified` for the 304
  path), and percent-decoding via `percent-encoding` (already in the dependency graph
  transitively). No explicit `..`-rejection was needed: `rust-embed`'s generated key set never
  contains a `..` segment, so `WebDist::get` already returns `None` for one, and the caller falls
  through to the shell exactly like an unmatched `ServeDir` path does.
- **`api/build.rs`, feature-gated on `CARGO_FEATURE_EMBED_WEB`**: `rust-embed`'s derive emits one
  `include_bytes!` per file, so rustc tracks content changes to files it already knows about but
  not files being added/removed — every `pnpm --filter web build` produces new content-hashed
  `_astro/*` filenames. `cargo:rerun-if-changed=../web/dist` closes that gap. Guarded on the feature
  rather than unconditional, since pointing `rerun-if-changed` at a path that may not exist (a
  fresh checkout before the frontend has ever been built, or simply the default build) forces an
  unconditional rebuild instead.
- **The open question this roadmap candidate carried — whether libgit2 honours `GIT_CONFIG_GLOBAL`
  for the bare-metal equivalent of the container's `[safe] directory = *` (#22) — is answered, not
  deferred further**, even though the systemd unit itself is still future work. Read directly from
  the vendored libgit2 1.9.6 source (`libgit2-sys-0.18.7+1.9.6/libgit2/src/libgit2/repository.c`):
  `config_path_global()` only consults `GIT_CONFIG_GLOBAL` when the repository is opened with
  `GIT_REPOSITORY_OPEN_FROM_ENV` — `repo/open.rs::open_named` uses `Repository::open_bare`, which
  does not set that flag, so **`GIT_CONFIG_GLOBAL` is not honoured** here. `$HOME/.gitconfig`
  (`sysdir.c`'s `find_global`) and `/etc/gitconfig` (`find_system`) *are* read regardless, and
  `validate_ownership_config()` looks up `safe.directory` through exactly that config stack.
  Conclusion for the deferred systemd unit: run the service as the user that **owns** the
  repositories, so the ownership check passes outright and no `safe.directory` entry — global,
  system, or otherwise — is needed at all.
- **Tests**: `api/src/assets.rs` unit-tests `Assets::resolve`'s precedence and `Assets::Dir::read`.
  `api/tests/embedded_assets_test.rs` (new, `#![cfg(feature = "embed-web")]`, runs against the real
  `web/dist`) covers root/asset/shell serving, content types, the `..` case, the ETag round-trip,
  an `AXGIT_STATIC_DIR` override, and site-meta injection reaching the embedded shell. One existing
  test's premise inverted: `static_shell_test.rs`'s "no static dir → 404" assumed no frontend is
  ever an option, which is no longer true once `embed-web` is on (no directory configured now falls
  back to the embedded copy) — split into a `#[cfg(not(feature = "embed-web"))]` 404 variant and a
  `#[cfg(feature = "embed-web")]` variant asserting the shell is served instead. Every other
  non-API-path test in `api/tests/` already configures a static dir or hits a matched route, so
  nothing else moves.
- **Size cost, measured**: a release build with `--features embed-web` is ~24.0 MB vs. ~20.0 MB
  without (`web/dist` itself is 4.0 MB across 132 files) — roughly a 1:1 add, expected since most of
  `web/dist`'s weight is already-compressed JS/`woff2`.
- Scope: this closes only the `embed-web` feature + serving path. Packaging (release tarball,
  systemd unit) and a Jenkins release stage stay a ROADMAP candidate, now with the ownership
  question above answered rather than open.

## #75 Single-binary deploy packaging: release tarball, systemd unit, Jenkins release stage

Closes the packaging half of the single-binary deploy candidate — #74 produced a complete `axgit`
executable, but nothing turned it into something installable: no tarball, no service definition,
and `Jenkinsfile` had never built a release artifact of any kind.

- **New `scripts/make-release.sh`**, following `make-fixtures.sh`'s shape
  (`set -euo pipefail`, `ROOT` from `$0`). Builds the frontend, then
  `cargo build --release --locked --features embed-web --target "$TARGET"`, and stages the
  resulting binary with `packaging/axgit.service`, `axgit.env.example`, `INSTALL.md`, and `LICENSE`
  into `release/axgit-$VERSION-$TARGET.tar.gz` + `release/SHA256SUMS`. `VERSION` is read from
  `api/Cargo.toml` rather than duplicated; when Jenkins' `TAG_NAME` is set (a tag build) the script
  asserts it matches `v$VERSION` and fails before building on a mismatch — a tag/manifest skew
  should stop the release, not ship a mislabelled tarball.
- **Target: `x86_64-unknown-linux-musl` only, built natively on the CI agent — not a Docker-stage
  build.** Matches the Dockerfile's static-linking posture (#22) without adding a Docker dependency
  to the Jenkins pipeline, which already assumes a bare Node/Rust toolchain. musl needs its own C
  compiler for the vendored C dependencies (`libgit2-sys`, `liblzma-sys`, `zstd-sys`, `bzip2-sys`
  all compile via the `cc` crate, same as the Dockerfile's alpine build) — the script requires
  `musl-gcc` on `PATH` and sets `CC_x86_64_unknown_linux_musl` explicitly rather than letting `cc`
  fall back to the host's glibc-targeting compiler, and checks the rustup target is installed
  first, failing with the exact remediation commands rather than a raw linker error. `TARGET` is
  overridable (`TARGET=aarch64-apple-darwin ./scripts/make-release.sh`) purely so the
  staging/tar/checksum logic can be exercised on a non-Linux dev machine — Jenkins always uses the
  musl default. arm64 is left off the release matrix; nothing in git-compose currently targets it,
  and it can be added as its own `TARGET` build later without changing the script.
  No stripping — the release profile matches the container build's, and stripping would invalidate
  #74's measured size figures without being asked for; left as a ROADMAP candidate instead.
- **`packaging/axgit.service`**: `Type=exec`, `EnvironmentFile=-/etc/axgit/axgit.env` (the `-`
  makes it optional — axgit's own defaults apply without one, matching `config.rs`'s all-env-var
  design). `User=`/`Group=` default to `git` with a comment pointing at #74's ownership finding —
  the operator changes it to whichever account owns `AXGIT_REPO_ROOT`, at which point the
  ownership check passes outright and no `safe.directory` entry is needed, on a bare-metal host any
  more than in the container. Hardened with the standard systemd sandboxing directives
  (`ProtectSystem=strict`, `PrivateTmp`, `RestrictNamespaces`, `SystemCallFilter=@system-service`,
  …) while leaving process spawning and network access open — axgit forks/execs `git` for
  archive/upload-pack (ARCHITECTURE.md's hybrid libgit2+exec policy), so those can't be sandboxed
  away. **`ProtectHome=read-only`, deliberately not `yes`**: libgit2 still reads
  `$HOME/.gitconfig` as part of the ownership-check config stack (#74), and a fully hidden home
  would make that lookup silently see nothing.
- **`packaging/axgit.env.example`** mirrors README.md's configuration table field-for-field (same
  order, same one-line descriptions) so the two don't drift independently — both ultimately
  describe `api/src/config.rs`'s `Config` struct.
- **`packaging/INSTALL.md`** ships inside the tarball itself (not just in the repo), since it's an
  operator-facing document needed at install time, on a host that may never have cloned this repo.
- **Jenkins trigger: `v*` tag builds only, via `archiveArtifacts`.** No dedicated release
  automation (pushing to a release host, notifying anyone) exists yet — this stage's job is
  producing and retaining the artifact, matching the existing pipeline's "test-only, artifacts
  archived" posture (`web/playwright-report/**` already works this way). `Jenkinsfile`'s `Release`
  stage runs after `Test` (`buildingTag()` + a `v.*` tag pattern), so a broken build never produces
  a tagged release artifact even if the tag itself was pushed.
- Verified end-to-end on macOS against the host triple (`TARGET=aarch64-apple-darwin`): tarball
  contents match exactly, `SHA256SUMS` round-trips, the `TAG_NAME` mismatch guard fails before
  building, and the extracted binary — run with no `AXGIT_STATIC_DIR` — serves the index shell,
  `/api/v1/repos`, and a content-hashed `_astro/*` asset, confirming the embedded frontend is what
  answers. The musl leg itself is exercised for the first time by the next `v*` tag build; local
  verification stops at the point only a Linux host can go further.
- No API contract change, no `api/src` change — this is packaging only.

## #76 CI coverage and a binary smoke check for the single-binary release path

Closes a gap #74/#75 left open: nothing between "code compiles" and "a `v*` tag ships a tarball"
had ever actually exercised the `embed-web` feature or run the binary it produces. `Jenkinsfile`'s
`Test` stage builds with default features only, so `api/src/assets.rs`'s `Embedded` arm,
`routes.rs`'s embedded fallback, and `api/tests/embedded_assets_test.rs`
(`#![cfg(feature = "embed-web")]`) plus `static_shell_test.rs`'s embed-gated variant were compiled
out of every ordinary build — the `Release` stage's `cargo build --features embed-web` on the next
pushed `v*` tag would have been the first build to ever touch that code. Compounding it,
`scripts/make-release.sh` never ran the binary it just built: a musl `cc`/link misconfiguration
producing a file that exists and is executable but immediately fails to start would have shipped
undetected.

- **New `Jenkinsfile` stage `Embedded build`**, sequential (not in the `Test` parallel block),
  positioned after `Test` and before `Release`, running on every build rather than gated to tags —
  a break should surface on the next ordinary commit, not on the next release attempt. It runs
  `pnpm --filter web build` (also the only place CI runs the *production* Astro build at all —
  Playwright's `webServer` in `web/playwright.config.ts` uses `pnpm dev`, not `astro build`, so
  this is a second gap the same stage happens to close), then
  `cargo clippy --features embed-web --all-targets -- -D warnings`, then `cargo test` scoped to
  `--lib --test embedded_assets_test --test static_shell_test` rather than the full integration
  suite: clippy `--all-targets` already proves every embed-gated file compiles under the feature,
  and re-running every other integration test (unaffected by `embed-web`) a second time would only
  spend the pipeline's 30-minute timeout on work the parallel `Test` stage already did. No musl
  toolchain is needed here — this stage builds `embed-web` for the host target, not
  `x86_64-unknown-linux-musl`; only `Release` cross/native-builds to musl.
- **`scripts/make-release.sh` smoke check**, inserted right after the existing `[[ -x "$BINARY" ]]`
  existence check: when the host can actually execute a `$TARGET` binary — same OS and CPU
  architecture, checked via small `target_os`/`target_arch` triple-matching helpers (a musl vs.
  glibc host libc difference doesn't matter for a statically-linked musl binary, so this correctly
  says yes for an `x86_64-unknown-linux-gnu` Jenkins agent building the default
  `x86_64-unknown-linux-musl` target, and for the `TARGET=aarch64-apple-darwin` local-verification
  case) — it runs `"$BINARY" --version` and asserts the output is exactly `axgit $VERSION`
  (confirmed against clap's actual `#[command(name = "axgit", version)]` output format,
  `api/src/config.rs`). A mismatch or non-zero exit fails the script before anything is staged or
  tarred. When the host can't run the target binary (the common case: any real Linux CI agent
  building for musl, or cross-target local verification), the check is skipped with a one-line
  explanation rather than failing the build — this can't become a hard requirement without buying
  the pipeline a matching runner or an emulation layer neither of which exist yet.
- Deliberately did **not** add a second Jenkins agent/emulation layer to make the smoke check run
  unconditionally in CI — the musl leg's first real execution therefore still happens only when a
  `v*` tag is built on a Linux agent (unchanged from #75's own caveat); this closes the "silently
  never compiled" gap, not the "never run on the actual release target" one.
- No API contract change, no `api/src` change.

## #77 Immutable `Cache-Control` for content-hashed `_astro/*` assets

Closes the ROADMAP candidate #74 and #75 both deliberately left open: neither `Assets::Dir`'s
`ServeDir` path nor `Assets::Embedded`'s `serve_embedded_file` (#74) ever set `Cache-Control` on a
static asset, so every load re-validates every content-hashed file with the browser even though
the filename itself already guarantees the content can never change.

- **One mode-independent layer, not two duplicated header-setting sites.** Astro's default
  `build.assets` config puts every content-hashed file (and nothing else — not `404.html`, not
  `favicon.svg`/`robots.txt`, not a page shell) under `/_astro/`, so `assets.rs::HASHED_ASSET_PREFIX`
  (`"/_astro/"`) plus a plain `starts_with` check is an exact test, not a heuristic guess at what's
  hashed. `assets::immutable_cache_for_hashed_assets` is a `tower::Layer`-compatible
  `axum::middleware::from_fn` handler wired in `routes.rs::build_router` at the outermost `Router`
  layer (alongside `TraceLayer`), so it sees the final response regardless of which arm
  (`Assets::Dir`'s `ServeDir::fallback(shell)` or `Assets::Embedded`'s
  `serve_embedded_file`-then-shell handler) produced it — no cfg-gating needed on the layer itself,
  only on the two arms that install it.
- **Applied only on a 200 or 304**, deliberately checked against `response.status()` after
  `next.run` rather than trusted from the request path alone: a `/_astro/*` request for a file that
  no longer exists in the current build (a stale link left over from a previous deploy, or simply a
  typo) falls through to the 404 shell like any other unmatched shape, and that 404 must never be
  marked immutable — a client that briefly hit a bad link would otherwise cache the miss for a
  year. Existing shells keep their own `no-cache` (`shell.rs`) untouched, since they're never under
  `/_astro/` at all.
- **Reused `handlers::IMMUTABLE_CACHE_CONTROL`** (`"public, max-age=31536000, immutable"`) rather
  than a second constant — it's the same header already used for full-sha-addressed API responses
  (docs/API.md), so the two are visibly the same freshness promise made for the same reason
  (content-addressed by the URL itself).
- **`serve_embedded_file`'s doc comment updated**, since the header it now gets is no longer set by
  that function directly — it previously documented deliberately matching `ServeDir`'s no-header
  default; that default itself is what changed here, for both modes at once.
- **Tests**: `api/tests/static_shell_test.rs` (`Assets::Dir` mode, using the fixture's existing
  `_astro/app.js`) covers a real hashed asset getting the header, the index shell *not* getting it
  (still `no-cache`), and a missing `_astro/*` path 404ing through the shell without inheriting it.
  `api/tests/embedded_assets_test.rs` (`Assets::Embedded` mode) parses a real `/_astro/*.js`
  reference out of the served index shell's own body rather than hardcoding a content hash that
  changes on every `pnpm --filter web build`, then checks both the 200 and the `If-None-Match` 304
  path carry the header.
- No API contract change, no `api/src` route/handler-shape change — a response header addition
  only.

## #78 Decided against stripping the release binary

Closes the ROADMAP candidate #75 left open, by deciding not to pursue it rather than by
implementing it. `scripts/make-release.sh` builds with the same `release` profile as the
Dockerfile (no `strip = true`), so a panic backtrace on either deploy shape still names real
functions. Stripping would shrink the binary (#74 measured ~24.0 MB unstripped; roughly 15–18 MB
was the expected range) at the cost of that symbol information — for a self-hosted, single-tenant
service where the operator debugging a panic *is* the person who'd read that backtrace, the
debugging value outweighs the size saving. Not revisiting unless a concrete need shows up (e.g. the
tarball size itself becoming a problem). No code change.

## #79 aarch64-unknown-linux-musl release leg

Closes the ROADMAP candidate #75 left open (arm64 excluded from the release matrix, revisit "if a
confirmed deploy target shows up"). `scripts/make-release.sh` now builds both
`x86_64-unknown-linux-musl` (native) and `aarch64-unknown-linux-musl` (cross) by default, producing
two tarballs and one `SHA256SUMS` covering both. Cross-compiling a musl target from `cc`-crate C
dependencies turned out to have several non-obvious failure modes, checked directly against `cc`
1.4.0's and `pkg-config` 0.3.33's source rather than assumed:

- **`bzip2-sys` no longer exists in the dependency graph** (`api/Cargo.lock`) — `bzip2 0.6`
  switched to the pure-Rust `libbz2-rs-sys` (`build = false`). The actual C dependency set is
  `libgit2-sys`, `libz-sys` (pulled in transitively by libgit2-sys, not by `flate2`, which uses
  `zlib-rs`), `liblzma-sys`, and `zstd-sys`. #75's text and the script's own comment both named
  `bzip2-sys` and omitted `libz-sys`; corrected in both places. No `cmake`/`bindgen` in the lock, so
  neither is a CI-agent prerequisite.
- **cc-rs's env-var lookup order is `CC_<triple-dashes>` > `CC_<triple_underscores>` > `TARGET_CC`
  > `CC`** (`target_envs`, cc 1.4.0). A script that exports `CC_<triple_underscores>` therefore
  shadows any `TARGET_CC` an operator might set — so the override mechanism is honouring a
  pre-set `CC_<triple_underscores>` in the environment, not adding a second variable. The dashed
  form is moot: `export "CC_aarch64-unknown-linux-musl=..."` isn't a valid bash identifier.
- **cc-rs's built-in cross prefix table only knows one aarch64 name**: `prefix_for_target` hardcodes
  `"aarch64-unknown-linux-musl" => Some("aarch64-linux-musl")` with no existence check, while the
  x86_64 musl case actually probes PATH (`find_working_gnu_prefix(["x86_64-linux-musl", "musl"])`).
  Practical effect: musl.cc/Homebrew `musl-cross` naming (`aarch64-linux-musl-gcc`) works with zero
  env vars; `messense/macos-cross-toolchains` naming (`aarch64-unknown-linux-musl-gcc`) does not and
  needs the explicit `CC_aarch64_unknown_linux_musl` export the script now does. The script probes
  both conventions, cross-only names first since the native (`musl-gcc`) case doesn't apply.
- **`CARGO_TARGET_<TRIPLE>_LINKER` is required for the cross leg, not optional.** Confirmed via
  `rustc --print target-spec-json --target aarch64-unknown-linux-musl`: no `linker` key,
  `linker-flavor: "gnu-cc"`. rustc drives the final link through PATH's `cc` regardless of
  self-contained musl crt/libc.a, and a host `cc` cannot link foreign-arch objects. Left unset on
  the native leg to keep the already-proven x86_64-on-x86_64 behaviour untouched. `AR_<triple>` is
  exported best-effort the same way (redundant under musl.cc naming, load-bearing under messense
  naming, and essential from a macOS host whose cctools `ar` can't index ELF); `RANLIB` is
  deliberately not set — cc 1.4.0 exposes a ranlib accessor but never calls it internally.
- **The single biggest hazard: never set `PKG_CONFIG_ALLOW_CROSS`, `PKG_CONFIG`, or
  `PKG_CONFIG_SYSROOT_DIR`.** `git2` doesn't enable libgit2-sys's `vendored` feature, so
  `libgit2-sys`'s `build.rs` always tries a system libgit2 via pkg-config first. The *only* reason
  today's build ends up vendored (statically built) at all is that `pkg-config` 0.3.33's
  `target_supported()` refuses to run when `host != target` unless one of those three variables
  overrides it. Setting any of them on a cross build flips libgit2-sys and libz-sys onto the host's
  glibc `.pc` files, producing a binary that looks statically linked but silently isn't. Documented
  as a prohibition next to the toolchain export in the script, not just here.
- **Two-pass structure**: the script now resolves and validates every target's toolchain (rustup
  target installed, C compiler found) before building any of them, so a missing aarch64 cross
  compiler fails immediately instead of after the x86_64 leg has already spent build time. Also
  wipes `release/` up front and writes `SHA256SUMS` once, from the explicit list of tarballs it
  produced — not a glob — so a stale file left over from an earlier invocation can never be
  checksummed alongside the current release.
- **Smoke check gained a runner ladder**, since qemu doesn't apply only to Jenkins:
  `CARGO_TARGET_<TRIPLE>_RUNNER` override (cargo's own per-target convention, not a bespoke
  variable) → native execution → `qemu-<arch>-static`/`qemu-<arch>` on `PATH` (no `-L <sysroot>`
  needed — the binary is statically linked musl, precisely the case naive qemu-user usage usually
  needs one for) → a registered, enabled `binfmt_misc` handler for the arch → skip. The skip path
  was deliberately kept as a hard "give up and say so" rather than "try executing it anyway and
  catch the failure" — a genuinely broken binary also fails to execute, so a blanket try/fallback
  would turn the exact hard failure #76 built this check to catch into a silent pass. Honest
  caveat: **on a stock x86_64 Jenkins agent with no `qemu-user-static` installed, the
  aarch64 binary is still built and shipped, just unexecuted** — the skip message was upgraded to
  `warning:` and names the package, but installing it is a recommended, not enforced, agent
  prerequisite (`Jenkinsfile`'s header comment says so explicitly).
- Both musl legs turned out to be fully buildable *and* runnable on the macOS/aarch64 dev machine
  used for this work — the messense cross toolchain for the aarch64 leg (exercising the
  explicit-env-var path above, not the one cc-rs already knows), and Docker's native (non-emulated)
  arm64 execution standing in for the smoke check via `CARGO_TARGET_..._RUNNER`. This retires the
  "the musl leg is first exercised by a v* tag build" limitation #75/#76 recorded — both legs are
  now locally verifiable before a release is tagged, at least for the C-toolchain and
  static-linking half of what a real Linux run would prove.
- `Jenkinsfile`'s pipeline `timeout` raised 30 → 45 minutes: a tag build's `Release` stage now pays
  two full `--release` builds (each including a fresh vendored libgit2/xz/zstd/zlib C build) instead
  of one, on top of the existing Test and Embedded build stages sharing the same pipeline-wide
  timeout.
- No API contract change, no `api/src` change — packaging only.

## #80 `Dockerfile` → `Containerfile` + symlink, base images qualified with their registry

`/Users/cookie/Workspaces/Projects/git-web` — the workspace previously used to build the cgit-based
`git-web` container image this project replaces — established two conventions this project's
`Dockerfile` (#22) hadn't picked up. Both ported over, container behavior otherwise unchanged:

- **`Containerfile` is now the real file; `Dockerfile` is a committed relative symlink to it**
  (`ln -s Containerfile Dockerfile`, `git ls-files -s Dockerfile` shows mode `120000`). git-web's
  own README documented `docker build` and `buildah build --file Containerfile` side by side, and
  podman/buildah look for `Containerfile` before `Dockerfile` by default — this makes both first-class
  without maintaining two copies. `docker build .` is unaffected (it follows the symlink
  transparently); no other file needed to change for this half.
- **Base-image `ARG`s spell out their registry** (`docker.io/library/node:24.11-alpine3.22`, etc.,
  up from bare `node:24.11-alpine3.22`). Docker already resolves an unqualified name to
  `docker.io/library/...`, so this changes nothing for `docker build`. podman/buildah instead
  consult `registries.conf`'s `unqualified-search-registries` and, lacking an unambiguous match,
  fall back to an interactive "which registry did you mean" prompt — which fails outright in any
  non-TTY build (CI, a script, `docker build`'s own BuildKit-in-buildah compatibility mode).
  Qualifying the reference removes the ambiguity entirely, matching git-web's Containerfile, which
  used `docker.io/library/alpine:3.22`/`docker.io/library/nginx:alpine3.22` throughout.

**Checked the rest of git-web's Containerfile against axgit's and found nothing else worth
porting** — recorded here so a future session doesn't rediscover these as gaps:

- `git-daemon` (apk package): git-web needed it because nginx calls
  `/usr/libexec/git-core/git-http-backend` over FastCGI for Smart HTTP. axgit's `smart_http.rs`
  execs `git upload-pack --stateless-rpc` directly and `archive.rs` execs `git archive` — both are
  part of the base `git` package alpine already installs; `git-http-backend` is never invoked.
- `VOLUME ["/srv/git"]`: git-web declared it so the image works even if a caller forgets to mount
  a real volume. For axgit that failure mode is the wrong one to hide — an anonymous, writable
  volume silently standing in for a forgotten `-v ...:/srv/git:ro` directly contradicts the
  `read_only: true` + `:ro` deployment posture (`docs/compose.example.yaml`). Left out on purpose;
  a missing mount should surface as "no repositories found" or a failed bind mount, not a quiet
  writable fallback.
- `/home/git/.gitconfig` + `HOME=/home/git`: needed by git-web's FastCGI `git-http-backend` call.
  axgit's single `/etc/gitconfig` (#22) already covers both the `git` exec call sites and libgit2,
  since both read the system-wide config; no per-user config or `HOME` is needed.
- Narrowing `safe.directory` from `*` to `/srv/git/*` (git-web's own setting): rejected again here,
  same reasoning as #22 — `AXGIT_REPO_ROOT` is a runtime-configurable env var, not a build-time
  constant, so a narrower pattern would silently break any deployment that points it somewhere
  else. `*` stays scoped to this single-purpose, read-only container.
- TLS termination, the letsencrypt volumes, `EXPOSE 80 443`: git-web served TLS itself via nginx;
  axgit has no nginx and TLS stays delegated to an external reverse proxy either way (#10),
  unaffected by this change.

**Left out of scope, not a container concern**: git-web's `nginx.conf`/`default.conf` also enforced
edge policy that has no axgit equivalent post-migration — a 405 on any method outside
GET/HEAD/POST, a 444 (connection close, no response) for AI-crawler/scraper/empty-`User-Agent`
patterns, and the HTTP→HTTPS redirect + certbot TLS termination itself. None of this is nginx-owned
config axgit can carry forward (axgit has no nginx layer), and reproducing it means it belongs in
the git-compose stack's external reverse proxy, not this repository. Flagged here rather than
silently dropped.
