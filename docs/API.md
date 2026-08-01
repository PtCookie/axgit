# Axgit API Spec (v1)

The **normative document** for the web ↔ api contract. Any commit that adds or changes an
endpoint must update this document too.

The machine-readable spec is `docs/openapi.json` (OpenAPI 3.1), **generated from code** via
utoipa annotations (DECISIONS.md #15). While the server is running it can be explored at
`/swagger-ui`, and the raw spec is at `/api/v1/openapi.json`. The spec covers schemas, parameters,
and status codes; this document covers the **semantic rules** the spec can't express (truncation
limits, refs longest-match, merge simplification, conditions under which a field is `null`).
**When they conflict, this document wins.**

- Base path: `/api/v1`
- All responses are `application/json` (except raw/archive/feed)
- Repository identifier `{repo}`: the repository name with the `.git` suffix removed
  (e.g. `git-compose`)
- The ref parameter accepts branch names, tag names, and commit shas. Defaults to HEAD if omitted.
- Fields in response objects are **always present as keys**, even when they have no value
  (serialized as `null`, never omitted).

## Common

### Error format

```json
{ "error": { "code": "repo_not_found", "message": "repository 'foo' not found" } }
```

| HTTP | code                | Situation |
| ---- | ------------------- | ---- |
| 404  | `repo_not_found`    | Repository does not exist |
| 404  | `ref_not_found`     | ref/sha resolution failed |
| 404  | `path_not_found`    | tree/blob path does not exist |
| 400  | `invalid_param`     | Malformed parameter |
| 403  | `read_only`         | Write attempt (e.g. receive-pack) |
| 404  | `not_found`         | No route matched under `/api/v1/...` (DECISIONS.md #16) |
| 500  | `internal`          | Server error (cause is logged only; the client gets a generic message) |

There is exactly one exception to this envelope: if the upload-pack request body exceeds 8 MiB,
axum's `DefaultBodyLimit` returns a plain-text `413`.

### Caching headers

- Responses whose URL includes a commit sha (immutable): `Cache-Control: public,
  max-age=31536000, immutable`. Since `{sha}` may also be a branch, tag, or abbreviated sha, this
  header is attached **only when the requested path value exactly matches the resolved commit's
  full sha as a string**. These responses have no `ETag`.
- Otherwise: `ETag` + `Cache-Control: no-cache`. A matching `If-None-Match` returns **304** (no
  body, `ETag`/`Cache-Control` still attached). The ETag value is **opaque** and its format is not
  part of the contract — the server derives it from the repo's HEAD sha + agefile mtime, so it
  changes right after a push. Comparison uses RFC 9110 weak comparison (ignoring the `W/` prefix).
- Exceptions:
  - `GET /api/v1/repos` (the list) isn't tied to a specific repo, so its ETag is a **hash of the
    response body**.
  - `raw` isn't subject to the server response cache, but non-sha requests still get an ETag
    (304 still saves transfer bytes).
  - `archive` uses a **weak ETag** (`W/"..."`). On a match, returns 304 without running
    `git archive`.
  - Smart HTTP endpoints are always `no-cache` and never use an ETag.

### Pagination (commit log)

Cursor-based. Pass the response's `next_cursor` (a commit sha) as the next request's `cursor`.
Default `limit=50`, max 100.

## Endpoints

### `GET /api/v1/repos`

Repository list. Equivalent to cgit's index.

```json
{
  "repos": [
    {
      "name": "git-compose",
      "section": "infra",
      "owner": "PtCookie",
      "description": "Compose project of Git server",
      "default_branch": "main",
      "last_modified": "2026-07-24T13:06:00+09:00"
    }
  ]
}
```

- `section`/`owner`/`description`: from the repo config's `[axgit]` section if present, else
  `[cgit]`.
- `last_modified`: from the agefile (`info/web/last-modified`), else HEAD authordate.
- `default_branch`/`last_modified`: `null` for an empty repository (no commits, no agefile).

### `GET /api/v1/repos/{repo}`

Repository summary. Equivalent to cgit's summary. List item fields plus `head` sha,
branch/tag counts, and the clone URL.

```json
{
  "name": "git-compose",
  "section": "infra",
  "owner": "PtCookie",
  "description": "Compose project of Git server",
  "default_branch": "main",
  "last_modified": "2026-07-24T13:06:00+09:00",
  "head": "<sha>",
  "branch_count": 2,
  "tag_count": 1,
  "clone_url": "https://git.example.com/git-compose.git"
}
```

- `name` through `last_modified`: same rules as the list item.
- `head`: HEAD commit sha. `null` for an empty repository (unborn HEAD) — this returns `200`, not
  404, with `default_branch`/`last_modified` also `null` and counts at 0.
- `clone_url`: `{clone_url_base}/{repo}.git`. `null` if `--clone-url-base`
  (`AXGIT_CLONE_URL_BASE`) is not configured.

### `GET /api/v1/repos/{repo}/refs`

```json
{
  "branches": [{ "name": "main", "target": "<sha>", "committed_at": "..." }],
  "tags": [{ "name": "v1.0.0", "target": "<sha>", "annotation": "...", "tagged_at": "..." }]
}
```

- Both `branches`/`tags` are sorted by name ascending. Both are `[]` for an empty repository.
- `branches[].target`: the branch tip commit sha. `committed_at`: the tip commit's authordate
  (RFC 3339).
- `tags[].target`: the **peeled commit sha** (for annotated tags, the target commit, not the tag
  object itself).
- `tags[].annotation`: the first line of the tag message. `tagged_at`: the tagger timestamp.
  **Both are `null` for lightweight tags.**

### `GET /api/v1/repos/{repo}/commits?ref=&path=&cursor=&limit=`

Commit log. When `path` is given, only commits that changed that path (cgit log's path filter).

```json
{
  "commits": [
    {
      "sha": "...", "summary": "...", "author": { "name": "...", "email_hash": "<sha256, avatar seed>" },
      "authored_at": "...", "parents": ["..."]
    }
  ],
  "next_cursor": "<sha|null>"
}
```

The raw email address is never exposed, only its hash. The frontend uses this hash as a seed to
render a locally generated avatar (DiceBear). `email_hash` is sha256 of the trimmed, lowercased
address (the gravatar approach).

- `ref`: branch/tag/sha, defaults to HEAD. `404 ref_not_found` on resolution failure.
- `limit`: default 50, allowed range 1–100. **0, values over 100, or non-integers are
  `400 invalid_param`** (not clamped).
- `cursor`: pass the previous response's `next_cursor` value verbatim. When `cursor` is given,
  `ref` is ignored and walking starts from that commit (**inclusive**). A malformed or
  non-existent commit is `400 invalid_param` (it's an opaque token, not a 404).
- `next_cursor`: the sha of the next page's first commit (after the `path` filter is applied).
  `null` if there are no more.
- `path`: a file or directory path. A path that doesn't exist returns an empty list, not a 404.
  A merge commit is included only when the path differs from **all** parents (an approximation of
  `git log -- <path>`'s default simplification — some side-branch commits may show up that
  wouldn't with the real `git log`).
- Empty repository (unborn HEAD): omitting `ref` returns `200` +
  `{"commits": [], "next_cursor": null}`. An explicit `ref` returns `404 ref_not_found`.
- `summary`: the commit message's first line. `summary`/`authored_at` are `null` for non-UTF-8
  messages or corrupted timestamps.

### `GET /api/v1/repos/{repo}/commits/{sha}`

Commit detail: full message, author/committer, parents, diffstat. A superset of the log entry
(`sha`/`summary`/`author`/`authored_at`/`parents` follow the same rules).

```json
{
  "sha": "<full sha>",
  "summary": "fix: update a",
  "message": "fix: update a\n\nfull body\n",
  "author": { "name": "...", "email_hash": "<sha256>" },
  "committer": { "name": "...", "email_hash": "<sha256>" },
  "authored_at": "2026-07-01T14:00:00+09:00",
  "committed_at": "2026-07-01T14:00:00+09:00",
  "parents": ["<sha>"],
  "diffstat": {
    "files": [
      { "path": "a.txt", "old_path": null, "status": "modified",
        "additions": 3, "deletions": 1, "binary": false }
    ],
    "files_changed": 1, "total_additions": 3, "total_deletions": 1
  }
}
```

- `{sha}`: branch/tag/sha (same rules as the ref parameter). `404 ref_not_found` on resolution
  failure.
- Diff is against the **first parent**: even for merge commits, only the diff against the first
  parent is shown. A root commit is diffed against an empty tree, so every file shows as `added`.
- `status`: `added` | `deleted` | `modified` | `renamed` | `copied` | `typechange`. Rename
  detection uses libgit2's default (50% similarity). `old_path` is non-null only for
  `renamed`/`copied`.
- Binary files have `binary: true` and `additions`/`deletions` of 0.
- `message`/`summary` are `null` for non-UTF-8 messages. The diffstat has no file count limit.

### `GET /api/v1/repos/{repo}/commits/{sha}/diff?path=`

A unified diff structured as JSON (file → hunk → line). File-level fields follow the same rules
as diffstat entries.

```json
{
  "sha": "<full sha>",
  "parent": "<first parent sha | null (root commit)>",
  "truncated": false,
  "files": [
    {
      "path": "a.txt", "old_path": null, "status": "modified",
      "additions": 1, "deletions": 1, "binary": false, "truncated": false,
      "hunks": [
        {
          "header": "@@ -1,2 +1,2 @@",
          "old_start": 1, "old_lines": 2, "new_start": 1, "new_lines": 2,
          "lines": [
            { "origin": " ", "content": "one",   "old_lineno": 1, "new_lineno": 1 },
            { "origin": "-", "content": "two",   "old_lineno": 2, "new_lineno": null },
            { "origin": "+", "content": "three", "old_lineno": null, "new_lineno": 2 }
          ]
        }
      ]
    }
  ]
}
```

- Diff basis (first parent, root against empty tree), `status`/rename/`old_path` rules are the
  same as commit detail.
- `origin`: only `"+"` | `"-"` | `" "`. Other libgit2 origins (e.g. EOF newline markers) are
  excluded. `content` has any trailing newline stripped; whichever of `old_lineno`/`new_lineno`
  doesn't apply is `null`.
- **Large diff limits**: 1000 rendered lines per file — beyond that, hunks are cut off (never
  mid-hunk) and the file gets `truncated: true` (since hunks aren't split mid-way, a single hunk
  longer than 1000 lines can leave `hunks` empty). 300 files max — the rest are omitted and the
  top-level `truncated: true` is set (see the commit detail's diffstat for the full file list).
  `additions`/`deletions` are always the full totals regardless of truncation.
- Binary files get `binary: true` + `hunks: []`.
- `path`: restricts the diff to a single file path (literal match, no glob support). A path that
  doesn't exist or wasn't changed in this commit returns `files: []`, not a 404.

### `GET /api/v1/repos/{repo}/tree/{ref}/{path...}`

Directory listing. Since `{ref}` may contain `/` (branch/tag names), the boundary with the path is
resolved via **refs longest-match**: the longest sequence of leading segments that matches an
existing branch/tag name is the ref (git ref rules guarantee `a` and `a/b` can't coexist, so this
is unambiguous); if there's no match, the first segment is treated as the ref (e.g. a commit sha).
blob/raw follow the same rule. Omitting `{path...}` means the root tree.

```json
{
  "sha": "<resolved full sha>",
  "path": "src",
  "entries": [
    { "name": "lib", "type": "tree", "mode": "040000", "size": null },
    { "name": "main.rs", "type": "blob", "mode": "100644", "size": 13 }
  ]
}
```

- `type`: `tree` | `blob` | `symlink` (mode 120000) | `commit` (submodule gitlink).
- `mode`: a 6-digit octal string. `size`: blob only, otherwise `null`.
- Sorting: trees first, then name ascending.
- `404 path_not_found` if the path doesn't exist or isn't a directory. `.`/`..`/empty segments in
  the path are `400 invalid_param`.
- If the request's `{ref}` matches the resolved full sha as a string, an immutable
  `Cache-Control` is attached (see the caching headers section — blob/raw/readme follow the same
  rule).

### `GET /api/v1/repos/{repo}/blob/{ref}/{path...}`

File metadata + content.

```json
{
  "sha": "<resolved full sha>",
  "path": "README.md",
  "mode": "100644",
  "size": 16,
  "binary": false,
  "too_large": false,
  "content": "# Axgit\n"
}
```

- `content`: UTF-8 text. Binary files (per libgit2's NUL heuristic, or non-UTF-8) get
  `binary: true` + `content: null` (pointing the client to raw instead).
- **Files over 1 MiB get `too_large: true` + `content: null`** (`size` is always the full value).
- Symlinks have mode `120000` and `content` = the link target path.
- `404 path_not_found` if the path doesn't exist or isn't a file (a directory or submodule).

### `GET /api/v1/repos/{repo}/raw/{ref}/{path...}`

Streams the raw file content. Equivalent to cgit's plain view. No size limit.

- `Content-Type`: detected by extension (mime_guess). Falls back to
  `text/plain; charset=utf-8` for text or `application/octet-stream` for binary if undetected.
- Since repository content is untrusted input, `X-Content-Type-Options: nosniff` is always
  attached.

### `GET /api/v1/repos/{repo}/readme?ref=`

Looks for a README and returns `{ "path": "...", "format": "markdown|rst|plain", "content": "..." }`.
Rendering (HTML conversion) is the frontend's responsibility. Only `markdown` is rendered;
`rst`/`plain` are shown as plain text (DECISIONS.md #11).

- Searches the root tree in order `README.md` → `README.rst` → `README.txt` → `README`,
  matching **case-insensitively**. Symlink, binary, or over-1-MiB candidates are skipped.
- `path` is the actual filename as it appears in the tree (case preserved). `404 path_not_found`
  if none is found.
- Defaults to HEAD if `ref` is omitted. `404 ref_not_found` for an empty repository (unborn HEAD)
  or resolution failure.
- `content` is subject to the same 1 MiB limit as blob.

### `GET /api/v1/repos/{repo}/blame/{ref}/{path...}`

Per-line-range attribution. `{ref}/{path...}` splitting and 404/400 rules are the same as
tree/blob/raw (refs longest-match, `.`/`..`/empty segments are 400, missing path/directory is 404).

```json
{
  "sha": "<resolved full sha>",
  "path": "src/main.rs",
  "binary": false,
  "too_large": false,
  "lines": 12,
  "ranges": [
    {
      "start_line": 1, "line_count": 3, "sha": "<commit sha>",
      "summary": "feat: initial", "author": { "name": "...", "email_hash": "<sha256>" },
      "authored_at": "2026-07-01T12:00:00+09:00"
    }
  ]
}
```

- `ranges` is sorted by `start_line` ascending (1-based) and covers the whole file with no gaps.
  `lines` is their total.
- Binary (per libgit2's NUL heuristic, or non-UTF-8) gets `binary: true`, **files over 1 MiB get
  `too_large: true`** (the same limit as blob's `BLOB_CONTENT_LIMIT`) — both cases return
  `ranges: []` + `lines: 0`. An empty file also gets `ranges: []` + `lines: 0`.
- `summary` is `null` for non-UTF-8 commit messages, `authored_at` is `null` for
  unrepresentable timestamps (same rules as the commit log). `author.email_hash` follows the same
  rule as the commit log (sha256 of the trimmed, lowercased email).
- No rename/copy tracking — only lines moved within the same file are attributed to their
  original commit, per libgit2's default.
- Implemented via **git2 `Repository::blame_file`** (not exec) — the path never touches a command
  line, and ARCHITECTURE.md already lists blame as git2's responsibility. If it proves slow on
  large histories, a `git blame --line-porcelain` exec fallback is a candidate for later (not
  currently implemented).
- Caching is the same as tree/blob: an immutable `Cache-Control` when `{ref}` matches the resolved
  full sha as a string, otherwise ETag (validator-based) + `no-cache` + 304.

### `GET /api/v1/repos/{repo}/archive/{ref}.{format}`

`format`: `tar.gz` | `zip`. Generated via `git archive` exec and streamed chunked (no
Content-Length). After ref resolution, **only the full sha is passed to exec** (user input never
reaches the command line).

- `Content-Type`: `application/gzip` | `application/zip`. Always
  `X-Content-Type-Options: nosniff`.
- `Content-Disposition: attachment; filename="{repo}-{safe_ref}.{format}"`. The archive's internal
  root directory (`--prefix`) is likewise `{repo}-{safe_ref}/`. `safe_ref` replaces any character
  outside `[A-Za-z0-9._-]` in the ref with `-` (e.g. `feature/x` → `feature-x`).
- `{ref}.{format}` is parsed by suffix match: `.tar.gz` first, then `.zip` (no conflict with `.`/
  `/` in the ref itself — a branch name ending in `.zip` is interpreted as a zip request). Any
  other suffix is `400 invalid_param`.
- If the request's `{ref}` matches the resolved full sha as a string, an immutable
  `Cache-Control` is attached; otherwise a weak ETag + `no-cache` (see the caching headers
  section). On a matching `If-None-Match`, returns 304 without spawning the git process.
- If the git process fails after streaming has started, the status code can no longer change, so
  the response is cut off mid-stream (the client sees a failed download; the server logs stderr).

### `GET /api/v1/repos/{repo}/feed.atom`

An Atom feed of the default branch's (HEAD's) most recent **20 commits**.
`Content-Type: application/atom+xml; charset=utf-8`.

- Feed: `<title>` = repo name, `<subtitle>` = description (only if present), `<id>` and the
  `rel="self"` link = this endpoint's absolute URL, `<updated>` = the latest commit's authordate
  (epoch if there are no commits).
- Entry: `<title>` = commit summary (`(no message)` if non-UTF-8), `<id>` =
  **`urn:sha1:{full sha}`** (stable regardless of host — avoids duplicate entries in feed
  readers), `<updated>` = authordate, `<author><name>` only (not even a hashed email), the
  `rel="alternate"` link = the commit detail API URL (**provisional** — will be swapped for the
  web UI's commit page route once that's finalized).
- The absolute URL base is reconstructed from `X-Forwarded-Proto` (default `http`) +
  `X-Forwarded-Host` → `Host` (default `localhost`) headers (no separate base URL config,
  DECISIONS.md #12).
- An empty repository (unborn HEAD) returns `200` with an entry-less feed, not a 404.
- ETag + `Cache-Control: no-cache` (see the caching headers section). Since the body embeds the
  base URL, the server response cache key includes the base URL too.

### `GET /api/v1/repos/{repo}/search?q=&type=&ref=&limit=`

Repository search: file content, file paths, or commit messages. Implemented as an in-process
git2 scan (not a `git grep` exec, not a persistent index — DECISIONS.md #26); every scan is
bounded by its own size budget, independent of `limit`.

```json
{
  "sha": "<full sha, or null for an empty repository>",
  "type": "content",
  "truncated": false,
  "files": [
    { "path": "src/main.rs", "lines": [{ "line": 12, "text": "..." }] }
  ],
  "commits": []
}
```

- `q`: **required**. A fixed string (not a regex), matched case-insensitively. Trimmed; empty
  after trimming or over 200 characters is `400 invalid_param`.
- `type`: `content` (default), `path`, or `message`. Any other value is `400 invalid_param`.
  - `content`: matches file content line by line. Binary files and files over the blob endpoint's
    1 MiB inline-content cap are skipped, same as the blob endpoint's classification. Symlinks are
    excluded (their "content" is a link target, not text).
  - `path`: matches the full file path (case-insensitive substring), no content is read. Symlinks
    are included; directories are not (there is nothing to match beyond the files under them).
  - `message`: matches a commit's full message (title + body), walking history from `ref`
    (or HEAD).
- `ref`: branch/tag/sha, defaults to HEAD. `404 ref_not_found` on resolution failure.
- `limit`: default 50, allowed range 1–100, same rules as the commit log's `limit` (not clamped).
  For `content`/`path` it caps the number of files returned; for `message` it caps the number of
  commits.
- `files`: populated for `type=content`/`type=path`; empty for `type=message`. Each entry's
  `lines` holds up to 10 matches (each truncated to 500 characters), empty for `type=path`.
- `commits`: populated for `type=message` only, using the same shape as a commit log entry
  (`CommitInfo`).
- `truncated`: `true` when either `limit` or an internal scan budget (20,000 tree entries scanned,
  32 MiB of blob content read for `content`, or 10,000 commits walked for `message`) was hit
  before the search finished — the results are a prefix, not necessarily the complete match set.
- Empty repository (unborn HEAD): omitting `ref` returns `200` with `"sha": null` and empty
  `files`/`commits`. An explicit `ref` returns `404 ref_not_found` — the same carve-out the
  commit log applies.
- Caching follows the commit-detail pattern: immutable only when `ref` is given and equals the
  resolved commit's full sha as a string; otherwise `ETag` + `Cache-Control: no-cache`.

## Smart HTTP (clone/fetch only)

Outside the API prefix, mapped directly to repository paths. Handled by spawning
`git upload-pack --stateless-rpc` (`--advertise-refs` for the advertise step, DECISIONS.md #13).
Smart protocol only — the dumb protocol (info/refs without a `service` parameter) isn't supported.

### `GET /{repo}.git/info/refs?service=git-upload-pack`

- `200` response: `Content-Type: application/x-git-upload-pack-advertisement`,
  `Cache-Control: no-cache`. The body is a pkt-line service header
  `001e# service=git-upload-pack\n0000` followed by upload-pack's ref advertisement.
- The client's `Git-Protocol` request header (e.g. `version=2`) is passed to upload-pack via the
  `GIT_PROTOCOL` env var — supporting protocol v2 negotiation. The service header pkt-line is
  attached the same way in v2.

### `POST /{repo}.git/git-upload-pack`

- The request body is upload-pack negotiation data
  (`application/x-git-upload-pack-request` — Content-Type isn't validated). A
  `Content-Encoding: gzip` body is decompressed by the server. Body limits: 8 MiB compressed
  (`413`), 64 MiB decompressed (`400`).
- `200` response: `Content-Type: application/x-git-upload-pack-result`,
  `Cache-Control: no-cache`, pack data streamed chunked. If upload-pack exits abnormally after
  streaming has started, the status code can't change and the stream ends early (the client sees
  an early EOF).

### Status codes

| Situation | Status | code |
| --- | --- | --- |
| Success | `200` | — |
| Missing/unsupported `service`, gzip decompression failure, decompressed size over the limit | `400` | `invalid_param` |
| Any `git-receive-pack`-related request (including info/refs' service param) | `403` | `read_only` |
| Repository not found, path missing the `.git` suffix | `404` | `repo_not_found` |
| Compressed body over the limit | `413` | — (axum's default response) |
| upload-pack spawn/advertise failure | `500` | `internal` |

Error bodies use the same JSON format as other endpoints (git clients ignore the body and only
look at the status code).
