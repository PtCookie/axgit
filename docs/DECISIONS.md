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
  way.

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
