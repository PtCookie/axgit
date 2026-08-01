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
