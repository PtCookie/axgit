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

- **Immutable** responses get `Cache-Control: public, max-age=31536000, immutable` and no `ETag`.
  This applies only when the request itself pins the resource to a full sha — the sha-bearing
  value, whether it's a path segment (`{sha}`, `{ref}`) or a query parameter (`ref`, `from`/`to`),
  must exactly match the resolved commit's full sha as a string, since it may otherwise be a
  branch, tag, or abbreviated sha. The commit log's opaque `cursor` also qualifies: it encodes a
  full-sha walk start. Each endpoint below states its own rule. Commit detail carries one extra
  condition beyond the full-sha match — see its `note` bullet above.
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

Cursor-based. Pass the response's `next_cursor` verbatim as the next request's `cursor`. The
cursor is an opaque token, not a commit sha — each page re-walks from the same fixed start commit
and skips ahead, which is what makes pagination lossless across side branches (see the `cursor`
row under `/commits` below, and `docs/DECISIONS.md` #37). Default `limit=50`, max 100.

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
- `branch_count`: **local branches only** — does not include `GET /refs`' `remote_branches`.
- `clone_url`: `{clone_url_base}/{repo}.git`. `null` if `--clone-url-base`
  (`AXGIT_CLONE_URL_BASE`) is not configured.

### `GET /api/v1/repos/{repo}/refs`

```json
{
  "branches": [{ "name": "main", "target": "<sha>", "committed_at": "..." }],
  "remote_branches": [{ "name": "origin/main", "target": "<sha>", "committed_at": "..." }],
  "tags": [{ "name": "v1.0.0", "target": "<sha>", "annotation": "...", "tagged_at": "..." }]
}
```

- `branches`/`remote_branches`/`tags` are each sorted by name ascending, each `[]` for an empty
  repository.
- `branches[].target`: the branch tip commit sha. `committed_at`: the tip commit's authordate
  (RFC 3339).
- `remote_branches`: remote-tracking branches (`refs/remotes/*`), same shape as `branches`
  (`name` is the full shorthand including the remote, e.g. `origin/main` — git has no API to split
  the remote name back out). **`[]` on essentially every repository axgit serves**: neither axgit
  nor the git-compose stack that populates it ever runs `git remote add`/`git fetch` against a
  served bare repository (pushes arrive over SSH only). This field exists for the rare case of a
  repository configured that way by hand, not for anything axgit itself produces. A remote's own
  symbolic `HEAD` (e.g. `origin/HEAD`) is omitted — it's an alias for another row, not a branch of
  its own.
- `tags[].target`: the **peeled commit sha** (for annotated tags, the target commit, not the tag
  object itself).
- `tags[].annotation`: the first line of the tag message. `tagged_at`: the tagger timestamp.
  **Both are `null` for lightweight tags.**
- For the tag object's own sha, its full message, the tagger, and the tag's un-peeled (one
  dereference) target, see `GET /tags/{name}` below.

### `GET /api/v1/repos/{repo}/tags/{name}`

Tag detail — the tag message body, tagger, and dereferenced target that `GET /refs` leaves out
(`tags[].annotation` there is only the first line, and `tags[].target` is already the fully
peeled commit).

```json
{
  "name": "v1.0.0",
  "tag_object": "<tag object sha, or null>",
  "object": { "sha": "<sha>", "type": "commit" },
  "target": "<peeled commit sha, or null>",
  "message": "Release v1.0.0\n\nSecond line.",
  "tagger": { "name": "...", "email_hash": "<sha256, avatar seed>" },
  "tagged_at": "..."
}
```

- `{name}`: tag name exactly as it appears under `refs/tags` — may itself contain `/`. **Only a
  real tag resolves here**: a branch name, a commit sha, or `HEAD` all answer `404 ref_not_found`
  (only `refs/tags/{name}` is consulted), which is an exception to this document's general "the ref
  parameter accepts branch names, tag names, and commit shas" rule.
- `tag_object`: the annotated tag object's own sha. `null` for a lightweight tag — a lightweight tag
  has no tag object, so `message`/`tagger`/`tagged_at` are `null` too, and `object.sha == target`.
- `object`: one dereference of the tag — the annotated tag's `object` header, or the lightweight
  tag's direct ref target. `object.type` is one of `commit`, `tree`, `blob`, `tag`. A **nested** tag
  (a tag pointing at another tag object) reports the inner tag here, with `type: "tag"` — `target`
  still fully peels through to the eventual commit.
- `target`: the fully peeled commit sha — the same value `GET /refs`' `tags[].target` reports.
  `null` when the tag chain never reaches a commit (a tag on a tree or blob), which is also exactly
  when an archive download is unavailable for this tag.
- `message`: the full tag message, trailing whitespace trimmed. `null` for a lightweight tag, a
  non-utf8 message, or a message that is empty or all whitespace.
- `tagger`/`tagged_at`: `null` for a lightweight tag, and for an annotated tag with no tagger line
  (git allows creating one without).
- **Never immutably cached**: the URL names a tag ref, not a sha, and a tag can be force-moved onto
  a different object without its name changing — always `ETag` + `Cache-Control: no-cache`.

### `GET /api/v1/repos/{repo}/commits?ref=&path=&cursor=&limit=&msg=`

Commit log. When `path` is given, only commits that changed that path (cgit log's path filter).

```json
{
  "commits": [
    {
      "sha": "...", "summary": "...", "body": "...",
      "author": { "name": "...", "email_hash": "<sha256, avatar seed>" },
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
- `cursor`: pass the previous response's `next_cursor` value verbatim. It's an opaque token
  encoding a fixed walk-start commit plus how many filtered commits to skip; `ref` is ignored when
  `cursor` is given. Every page re-walks from that same start commit, so pagination matches an
  unpaginated walk exactly — no side-branch commit pending at a page boundary is ever dropped. A
  malformed cursor, or one whose skip count exceeds 100,000, is `400 invalid_param` (not a 404).
  The walk cost of page *K* is proportional to `K × limit`. Because the start is fixed in the
  token, every `cursor` page is immutably cacheable (see "Caching headers").
- `next_cursor`: opaque; `null` if there are no more pages after the `path` filter is applied.
- `path`: a file or directory path. A path that doesn't exist returns an empty list, not a 404.
  A merge commit is included only when the path differs from **all** parents (an approximation of
  `git log -- <path>`'s default simplification — some side-branch commits may show up that
  wouldn't with the real `git log`).
- Empty repository (unborn HEAD): omitting `ref` returns `200` +
  `{"commits": [], "next_cursor": null}`. An explicit `ref` returns `404 ref_not_found`.
- `summary`: the commit message's first line. `summary`/`authored_at` are `null` for non-UTF-8
  messages or corrupted timestamps.
- `msg`: `0`/`1`/`true`/`false`, default off (cgit's `showmsg=1`). When on, each entry also carries
  `body` — the message past the summary line, trimmed. **The `body` key itself is omitted**, not
  set to `null`, when `msg` is off, when a commit has no body, or for a non-UTF-8 message — so its
  presence alone tells the caller whether there's anything to show. Any other value is
  `400 invalid_param`. Git notes are not included (no analogue of cgit's `showmsg` note row).
- **Immutable caching** when the request pins the walk start to a full sha: `cursor` always does
  (the token encodes one), and `ref` does when it equals the resolved commit's full sha as a
  string. Everything else — no `ref`, a symbolic `ref`, or the empty-repository page — is `ETag` +
  `Cache-Control: no-cache`. `path`, `limit`, and `msg` don't affect this: for a fixed walk start
  the page is a pure function of the request. `msg` is part of the cache key, so a `msg=1` response
  never collides with the default (bodyless) one for the same request.

### `GET /api/v1/repos/{repo}/commits/{sha}`

Commit detail: full message, author/committer, parents, diffstat. A superset of the log entry
(`sha`/`summary`/`author`/`authored_at`/`parents` follow the same rules).

```json
{
  "sha": "<full sha>",
  "summary": "fix: update a",
  "message": "fix: update a\n\nfull body\n",
  "note": null,
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
- `note`: the `git notes` message attached to this commit on the repository's default notes ref
  (`refs/notes/commits`, or `core.notesRef` when set) — only the default ref is read, unlike cgit's
  `notes.displayRef`/multi-ref concatenation. `null` when there is no note, no notes ref at all, or
  the note is non-UTF-8 or blank.
- Caching: a commit carrying a `note` is never eligible for immutable caching, even when `{sha}` is
  the resolved full sha — a note can change without the commit sha changing, so the response falls
  back to `ETag` + `no-cache` (see "Caching headers").

### `GET /api/v1/repos/{repo}/commits/{sha}/diff?path=&context=&ignorews=`

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
- `context`: integer `0..=100`, default `3` (libgit2's own default). Parsed manually — an invalid
  value (out of range or non-numeric) is `400 invalid_param`, never clamped.
- `ignorews`: `0`/`1`/`true`/`false`, default off. Maps to `git diff --ignore-all-space`. A file
  whose only changes are whitespace still appears (`status` stays `modified`, not dropped from
  `files`) but with `hunks: []` and `additions`/`deletions` both `0` — the same shape as a binary
  file, since libgit2 recomputes line stats from the whitespace-ignoring patch too. Any other value
  is `400 invalid_param`.

### `GET /api/v1/repos/{repo}/diff?from=&to=&path=&context=&ignorews=&stat=`

Arbitrary two-revision diff — `git diff <from> <to>`, a plain tree-to-tree comparison, **not** a
merge-base `A...B` diff. Same file/hunk/line shape as the per-commit diff, plus an uncapped
`diffstat` (the full file list, same rule as commit detail's).

```json
{
  "from": "<full sha | null>",
  "to": "<full sha>",
  "truncated": false,
  "diffstat": { "files": [ /* DiffStatFile, uncapped */ ], "files_changed": 1, "total_additions": 1, "total_deletions": 1 },
  "files": [ /* same FileDiff shape as the per-commit diff */ ]
}
```

- `from`: old side of the comparison. Defaults to `to`'s first parent (the empty tree for a root
  commit, in which case `from` is `null`). Accepts anything `resolve_commit` does: branch, tag,
  sha, or a `revparse_single` expression (`main~1`, `v1.0^{}`). `404 ref_not_found` naming the
  value on resolution failure.
- `to`: new side. Defaults to `HEAD`. Same acceptance/error rules as `from`.
- Both `from` and `to` are echoed back **resolved to their full sha** — not the requested string —
  so a symbolic `from`/`to` can be linked to its own commit page.
- `?to=X` alone (no `from`) produces `files` byte-identical to `GET /commits/X/diff` with the same
  `path`/`context`/`ignorews`: this endpoint is a strict superset of the per-commit diff.
- `path`, `context`, `ignorews`: same rules as the per-commit diff.
- `stat`: `0`/`1`/`true`/`false`, default off (cgit's `dt=2`). When on, hunk rendering is skipped
  entirely — `files` is always `[]` and `truncated` is always `false` (there is nothing left to
  cap), while `diffstat` is computed exactly as it would be otherwise — the same uncapped file
  list as the commit detail's diffstat. `context`/`ignorews` have no effect on `diffstat` and are
  ignored in this mode. Any other value is `400 invalid_param`.
- **Immutable caching** requires every side actually present in the request to resolve to itself as
  a full sha string; an omitted `from` inherits whatever `to` resolved to (so `?to=<full sha>` alone
  is immutable, `?from=<full sha>&to=<full sha>` is immutable, `?to=main` is not). `stat` is part of
  the cache key, so a stat-only response never collides with the full diff for the same revisions.

### `GET /api/v1/repos/{repo}/rawdiff?from=&to=&path=&context=&ignorews=`

Plain unified diff (`text/plain; charset=utf-8`) between two revisions, for `git apply`. Same
`from`/`to`/`path`/`context`/`ignorews` semantics as `GET /diff` — a two-dot tree comparison, not a
merge-base `...` diff.

- **No size limit** — unlike the structured JSON diffs, `MAX_FILE_DIFF_LINES`/`MAX_DIFF_FILES` do
  not apply here: a truncated patch would be a corrupt one, which defeats the endpoint's purpose.
- `X-Content-Type-Options: nosniff` is always set (repository content is untrusted input).
- Caching rules (immutable vs. `ETag`) are identical to `GET /diff`.

### `GET /api/v1/repos/{repo}/patch?from=&to=&path=`

`git format-patch`-style mbox series (`text/plain; charset=utf-8`) for the commit range
`(from, to]`, for `git am`.

- **`from` is excluded** — the opposite of `/diff`/`/rawdiff`'s `from`, which is the *other side* of
  a tree comparison. `from` omitted renders a single patch for `to` alone (`(from, to]` with an
  implicit empty `from` is just `{to}`).
- Every commit in the range is diffed against its **own first parent**, including merges. `git
  format-patch` itself skips merge commits entirely; axgit stays consistent with every other diff
  in this API (all first-parent) instead of matching that behavior.
- `path` restricts every commit's diff in the series to one file, same rule as elsewhere.
- **No `context`/`ignorews`** — format-patch output is meant to be applied, not read, so display
  options don't apply; any such query params are silently ignored, like any stray param elsewhere.
- **Capped, not truncated**, at 100 commits: a range exceeding the limit is `400 invalid_param`
  rather than a silently shortened (and therefore misleading) series.
- Includes the commit author's **name and a plain email address** in each patch's `From:` header —
  required for `git am` to preserve authorship. This is the **one exception** to "email addresses
  are never exposed" (docs/DECISIONS.md #8): the same data is already served, unauthenticated, by
  `git clone` over Smart HTTP, so nothing new is disclosed; what changes is that a GET response is
  easier to crawl than a pack. Mitigated with `Content-Disposition: inline; filename="..."` and
  `X-Robots-Tag: noindex, nofollow`; see docs/DECISIONS.md #38.
- Caching rules (immutable vs. `ETag`) are identical to `GET /diff`.

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
    { "name": "lib", "type": "tree", "mode": "040000", "size": null, "target": null },
    { "name": "main.rs", "type": "blob", "mode": "100644", "size": 13, "target": null },
    { "name": "readme-link", "type": "symlink", "mode": "120000", "size": null, "target": "../README.md" }
  ]
}
```

- `type`: `tree` | `blob` | `symlink` (mode 120000) | `commit` (submodule gitlink).
- `mode`: a 6-digit octal string. `size`: blob only, otherwise `null`.
- `target`: the link target path, symlinks only (`null` otherwise, and for a non-UTF-8 target or one
  larger than 4096 bytes). Stored verbatim and **relative to the entry's own directory** — it is
  never resolved server-side, so a `../` prefix reaches the client intact; clients resolve it
  themselves. The blob endpoint exposes the same value as `content`.
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
      "authored_at": "2026-07-01T12:00:00+09:00", "orig_path": null
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
- **Whole-file renames are tracked**, the same way `git blame` follows them by default (libgit2
  runs its own rename-similarity diff internally). A range attributed to a commit where the file
  still had a different path gets `orig_path` set to that path; `orig_path` is `null` when the
  path is unchanged (or not valid UTF-8). Line-level move/copy tracking (`git blame -M`/`-C`) is
  **not** supported — libgit2's equivalent flags are reserved but unimplemented upstream, so this
  would need an exec fallback.
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
  `rel="alternate"` link = the web UI's commit page (`/{repo}/commit/{sha}`).
- The absolute URL base is reconstructed from `X-Forwarded-Proto` (default `http`) +
  `X-Forwarded-Host` → `Host` (default `localhost`) headers (no separate base URL config,
  DECISIONS.md #12). The repository name is percent-encoded wherever it appears in an emitted URL.
- An empty repository (unborn HEAD) returns `200` with an entry-less feed, not a 404.
- ETag + `Cache-Control: no-cache` (see the caching headers section). Since the body embeds the
  base URL, the server response cache key includes the base URL too.

### `GET /api/v1/repos/{repo}/search?q=&type=&ref=&limit=`

Repository search: file content, file paths, commit messages, author/committer names, or a
rev-list range expression. Implemented as an in-process git2 scan (not a `git grep`/`git log` exec,
not a persistent index — DECISIONS.md #26/#46); every scan is bounded by its own size budget,
independent of `limit`.

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

- `q`: **required**. Trimmed; empty after trimming or over 200 characters is `400 invalid_param`.
  A fixed string (not a regex), matched case-insensitively, for every `type` except `range`, where
  it is instead a case-sensitive rev-list expression (see below).
- `type`: `content` (default), `path`, `message`, `author`, `committer`, or `range`. Any other
  value is `400 invalid_param`.
  - `content`: matches file content line by line. Binary files and files over the blob endpoint's
    1 MiB inline-content cap are skipped, same as the blob endpoint's classification. Symlinks are
    excluded (their "content" is a link target, not text).
  - `path`: matches the full file path (case-insensitive substring), no content is read. Symlinks
    are included; directories are not (there is nothing to match beyond the files under them).
  - `message`: matches a commit's full message (title + body), walking history from `ref`
    (or HEAD).
  - `author`/`committer`: matches the commit's author/committer **name only**, never the email —
    axgit never exposes a raw email address in any response (only `email_hash`), and matching the
    email here would turn the search box into a confirm/deny oracle for it. A deliberate difference
    from cgit's `--author=`, which matches `Name <email>`. Walks history from `ref` (or HEAD), same
    as `message`.
  - `range`: `q` is a `git log`-style rev-list expression instead of a text filter — the query
    *selects* commits, it doesn't match against them. Space-separated tokens, each one of:
    `A..B` (commits reachable from `B` but not `A`; an empty side means `HEAD`), `A...B`
    (symmetric difference — reachable from either but not both, hiding every merge base), `^X`
    (excludes everything reachable from `X`), or a bare revision (walks everything reachable from
    it). A token starting with `-` is rejected as `400 invalid_param` (a `git log` flag, not a
    revision). Every revision is resolved the same way `ref` is; one that doesn't resolve is
    `404 ref_not_found`. `ref` itself is not part of the walk for this type — it only determines
    the response's `sha` — so `q` alone must name everything to include.
- `ref`: branch/tag/sha, defaults to HEAD. `404 ref_not_found` on resolution failure.
- `limit`: default 50, allowed range 1–100, same rules as the commit log's `limit` (not clamped).
  For `content`/`path` it caps the number of files returned; for every commit-producing type
  (`message`, `author`, `committer`, `range`) it caps the number of commits.
- `files`: populated for `type=content`/`type=path`; empty otherwise. Each entry's `lines` holds
  up to 10 matches (each truncated to 500 characters), empty for `type=path`.
- `commits`: populated for `type=message`/`author`/`committer`/`range`; empty otherwise. Same shape
  as a commit log entry (`CommitInfo`).
- `truncated`: `true` when either `limit` or an internal scan budget (20,000 tree entries scanned,
  32 MiB of blob content read for `content`, or 10,000 commits walked for any commit-producing
  type) was hit before the search finished — the results are a prefix, not necessarily the
  complete match set.
- Empty repository (unborn HEAD): omitting `ref` returns `200` with `"sha": null` and empty
  `files`/`commits`. An explicit `ref` returns `404 ref_not_found` — the same carve-out the
  commit log applies.
- Caching follows the commit-detail pattern: immutable only when `ref` is given and equals the
  resolved commit's full sha as a string; otherwise `ETag` + `Cache-Control: no-cache`. **Never
  immutable for `type=range`**, even with a full-sha `ref` — the result depends on the revisions
  named in `q` (e.g. `main~5..main`), which can move independently of `ref`.

### `GET /api/v1/repos/{repo}/stats?ref=&period=&limit=`

Commit-activity statistics: commit counts bucketed by time period, plus a per-author breakdown.
cgit's `stats` page. Implemented as an in-process git2 revwalk (not a `git log` exec, not a
persistent index — DECISIONS.md #28), bounded by the same kind of scan budget as search.

```json
{
  "sha": "<full sha, or null for an empty repository>",
  "period": "month",
  "truncated": false,
  "author_count": 23,
  "buckets": [{ "start": "2025-09-01T00:00:00+00:00", "commits": 12 }],
  "authors": [
    { "author": { "name": "...", "email_hash": "..." }, "commits": 120, "buckets": [3, 0, 7] }
  ],
  "others": { "count": 12, "commits": 84, "buckets": [1, 0, 3] }
}
```

- `ref`: branch/tag/sha, defaults to HEAD. `404 ref_not_found` on resolution failure.
- `period`: `week`, `month` (default), `quarter`, or `year`. Any other value is
  `400 invalid_param`.
- The response always covers **12 buckets**, regardless of `period` — the window is anchored on
  the **resolved commit's authordate** (not the request time), so a full-sha `ref` yields a
  deterministic response and is eligible for immutable caching, same as commit detail. The last
  bucket is the period containing that authordate; the first is 11 periods before it.
- `buckets[].start`: the bucket's start instant, UTC, RFC 3339. Ascending order (oldest first).
  Week buckets start on Monday 00:00 UTC; month/quarter/year buckets start on the 1st.
- `buckets[].commits`: total commits in that bucket, **including** any authors cut by `limit` —
  bucket totals are never affected by the author cap.
- `authors[].buckets` is a **parallel array** to the top-level `buckets`: same length and order,
  so `authors[i].buckets[j]` is that author's commit count in `buckets[j]`.
- A commit older than the 12-bucket window is excluded entirely. A commit authored *after* the
  window's end (possible with out-of-order authordates across merged branches) is clamped into
  the last bucket rather than dropped.
- `limit`: default 50, allowed range 1–100, same rules as the commit log's `limit` (not clamped).
  Caps the number of `authors` rows returned, most active first; `author_count` reports the full
  distinct-author count within the window even when `authors` is cut shorter.
- `others`: the authors past `limit`, aggregated rather than dropped — `null` when the cut removed
  nothing. `others.buckets` is a parallel array to the top-level `buckets`, same as
  `authors[].buckets`, and `others.count` equals `author_count - authors.length`. Every `authors`
  row plus `others` therefore reconciles with `buckets[].commits`, column by column.
- `truncated`: `true` when either the commit scan budget (20,000 commits walked) or `limit` cut
  the results before finishing — the same two-cause rule search's `truncated` uses. `others` is the
  precise signal for the `limit` cause: `others != null` means `limit` cut authors, while
  `truncated && others == null` means the scan budget alone stopped the walk.
- Empty repository (unborn HEAD): omitting `ref` returns `200` with `"sha": null`, empty
  `buckets`/`authors`, `author_count: 0`, `"others": null`. An explicit `ref` returns
  `404 ref_not_found` — the same carve-out the commit log and search apply.
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
