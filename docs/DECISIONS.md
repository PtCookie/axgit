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
- Rename/copy tracking (the equivalent of `git blame --follow`) is not implemented — only
  within-file line moves are attributed, per libgit2's default. This is separate from commit
  detail/diff's rename detection (an existing policy predating #8).

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
