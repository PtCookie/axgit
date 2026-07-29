# Axgit API 명세 (v1 draft)

web ↔ api 간 유일한 계약 문서. 엔드포인트를 추가/변경하는 커밋은 반드시 이 문서를 함께 갱신한다.

- Base path: `/api/v1`
- 모든 응답은 `application/json; charset=utf-8` (raw/archive/feed 제외)
- 저장소 식별자 `{repo}`: `.git` 접미사를 제외한 저장소 이름 (예: `git-compose`)
- ref 파라미터는 브랜치명, 태그명, 커밋 sha 모두 허용. 생략 시 HEAD.

## 공통

### 에러 형식

```json
{ "error": { "code": "repo_not_found", "message": "repository 'foo' not found" } }
```

| HTTP | code                | 상황 |
| ---- | ------------------- | ---- |
| 404  | `repo_not_found`    | 저장소 없음 |
| 404  | `ref_not_found`     | ref/sha 해석 실패 |
| 404  | `path_not_found`    | tree/blob 경로 없음 |
| 400  | `invalid_param`     | 파라미터 형식 오류 |
| 403  | `read_only`         | receive-pack 등 쓰기 시도 |
| 501  | `not_implemented`   | 아직 구현되지 않은 엔드포인트 (스케폴딩 기간 한정, v1 완성 시 제거) |

### 캐싱 헤더

- 커밋 sha가 URL에 포함된 응답(불변): `Cache-Control: public, max-age=31536000, immutable`.
  `{sha}` 자리에는 브랜치/태그/축약 sha도 올 수 있으므로, **요청 경로의 값이 해석된 커밋의
  full sha와 정확히 일치할 때만** 이 헤더가 붙는다 (commit 상세/diff에 구현됨).
- 그 외: `ETag`(해당 repo HEAD sha 기반) + `Cache-Control: no-cache`, 조건부 요청 시 304
  — **미구현**, 응답 캐시 도입 시점에 일괄 구현 예정 (ROADMAP 참고).

### 페이지네이션 (commit log)

cursor 방식. 응답의 `next_cursor`(커밋 sha)를 다음 요청의 `cursor`로 전달. 기본 `limit=50`, 최대 100.

## 엔드포인트

### `GET /api/v1/repos`

저장소 목록. cgit index에 해당.

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

- `section`/`owner`/`description`: repo config `[axgit]` 우선, 없으면 `[cgit]` 섹션.
- `last_modified`: agefile(`info/web/last-modified`), 없으면 HEAD authordate.
- `default_branch`/`last_modified`: 빈 저장소(커밋 없음, agefile 없음)에서는 `null`.

### `GET /api/v1/repos/{repo}`

저장소 요약. cgit summary에 해당. 목록 항목 필드 + `head` sha, 브랜치/태그 개수, clone URL.

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

- `name`~`last_modified`: 목록 항목과 동일한 규칙.
- `head`: HEAD 커밋 sha. 빈 저장소(unborn HEAD)는 `null` (404가 아니라 200으로 응답,
  이때 `default_branch`/`last_modified`도 `null`, 카운트는 0).
- `clone_url`: `{clone_url_base}/{repo}.git`. `--clone-url-base`(`AXGIT_CLONE_URL_BASE`)가
  설정되지 않았으면 `null`.

### `GET /api/v1/repos/{repo}/refs`

```json
{
  "branches": [{ "name": "main", "target": "<sha>", "committed_at": "..." }],
  "tags": [{ "name": "v1.0.0", "target": "<sha>", "annotation": "...", "tagged_at": "..." }]
}
```

- `branches`/`tags` 모두 이름 오름차순. 빈 저장소는 둘 다 `[]`.
- `branches[].target`: 브랜치 tip 커밋 sha. `committed_at`: tip 커밋 authordate (RFC 3339).
- `tags[].target`: **peel된 커밋 sha** (annotated 태그도 태그 오브젝트가 아닌 대상 커밋).
- `tags[].annotation`: 태그 메시지 첫 줄. `tagged_at`: tagger 시각.
  **경량(lightweight) 태그는 둘 다 `null`.**

### `GET /api/v1/repos/{repo}/commits?ref=&path=&cursor=&limit=`

커밋 로그. `path` 지정 시 해당 경로를 변경한 커밋만 (cgit log의 path filter).

```json
{
  "commits": [
    {
      "sha": "...", "summary": "...", "author": { "name": "...", "email_hash": "<sha256, 아바타 seed용>" },
      "authored_at": "...", "parents": ["..."]
    }
  ],
  "next_cursor": "<sha|null>"
}
```

이메일 원문은 노출하지 않고 해시만 제공한다. 프론트엔드가 이 해시를 seed로 로컬 생성 아바타(DiceBear)를 렌더링한다.
`email_hash`는 trim + lowercase 정규화 후 sha256 (gravatar 방식).

- `ref`: 브랜치/태그/sha, 생략 시 HEAD. 해석 실패 시 `404 ref_not_found`.
- `limit`: 기본 50, 허용 범위 1–100. **0, 100 초과, 정수가 아닌 값은 `400 invalid_param`** (clamp하지 않음).
- `cursor`: 이전 응답의 `next_cursor` 값을 그대로 전달. cursor가 주어지면 `ref`는 무시되고
  해당 커밋부터(**포함**) 걷는다. 형식이 잘못되었거나 존재하지 않는 커밋이면 `400 invalid_param`
  (opaque 토큰이므로 404가 아님).
- `next_cursor`: 다음 페이지 첫 커밋의 sha (`path` 필터 적용 후 기준). 더 없으면 `null`.
- `path`: 파일 또는 디렉터리 경로. 존재하지 않는 경로는 404가 아니라 빈 목록.
  merge 커밋은 해당 경로가 **모든** 부모와 다를 때만 포함 (`git log -- <path>` 기본
  simplification의 근사 — side branch의 커밋이 일부 더 보일 수 있음).
- 빈 저장소(unborn HEAD): `ref` 생략 시 `200` + `{"commits": [], "next_cursor": null}`.
  명시적 `ref`는 `404 ref_not_found`.
- `summary`: 커밋 메시지 첫 줄. `summary`/`authored_at`은 non-utf8 메시지·손상된 타임스탬프일 때 `null`.

### `GET /api/v1/repos/{repo}/commits/{sha}`

커밋 상세: 전체 메시지, author/committer, 부모, diffstat. 로그 항목의 상위집합
(`sha`/`summary`/`author`/`authored_at`/`parents`는 동일 규약).

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

- `{sha}`: 브랜치/태그/sha (ref 파라미터와 동일 규약). 해석 실패 시 `404 ref_not_found`.
- diff 기준은 **첫 부모**: merge 커밋도 첫 부모와의 diff만 보여준다. root 커밋은 empty tree와
  비교하므로 전 파일 `added`.
- `status`: `added` | `deleted` | `modified` | `renamed` | `copied` | `typechange`.
  rename 감지는 libgit2 기본값(유사도 50%). `old_path`는 `renamed`/`copied`일 때만 non-null.
- 바이너리 파일은 `binary: true`에 `additions`/`deletions`는 0.
- `message`/`summary`는 non-utf8 메시지일 때 `null`. diffstat에는 파일 수 상한이 없다.

### `GET /api/v1/repos/{repo}/commits/{sha}/diff?path=`

unified diff를 구조화한 JSON (파일 → hunk → line). 파일 레벨 필드는 diffstat 항목과 동일 규약.

```json
{
  "sha": "<full sha>",
  "parent": "<첫 부모 sha | null(root 커밋)>",
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

- diff 기준(첫 부모, root는 empty tree), `status`/rename/`old_path` 규약은 커밋 상세와 동일.
- `origin`: `"+"` | `"-"` | `" "`만. libgit2의 기타 origin(EOF 개행 마커 등)은 제외된다.
  `content`는 후행 개행 제거, `old_lineno`/`new_lineno`는 해당 없는 쪽이 `null`.
- **대형 diff 상한**: 파일당 렌더 라인 1000줄 — 초과 시 hunk 단위로 잘라내고 파일의
  `truncated: true` (hunk를 중간에서 자르지 않으므로, 단일 hunk가 1000줄을 넘으면 `hunks`가
  빌 수 있다). 파일 수는 300개 — 초과분은 생략하고 최상위 `truncated: true` (전체 파일 목록은
  커밋 상세의 diffstat 참조). `additions`/`deletions`는 truncation과 무관하게 전체 값.
- 바이너리 파일은 `binary: true` + `hunks: []`.
- `path`: 단일 파일 경로로 diff 제한 (리터럴 매칭, glob 미지원). 존재하지 않거나 이 커밋에서
  변경되지 않은 경로는 404가 아니라 `files: []`.

### `GET /api/v1/repos/{repo}/tree/{ref}/{path...}`

디렉토리 목록. `entries[]`: `name`, `type`(`blob`|`tree`|`commit`(submodule)|`symlink`), `mode`, `size`(blob만).

### `GET /api/v1/repos/{repo}/blob/{ref}/{path...}`

파일 메타 + 내용. 텍스트는 `content`(utf-8), 바이너리는 `binary: true`에 내용 생략(raw로 유도).
`size`, `too_large`(상한 초과 시 내용 생략) 포함.

### `GET /api/v1/repos/{repo}/raw/{ref}/{path...}`

파일 원문을 감지한 MIME 타입으로 스트리밍. cgit plain view에 해당.

### `GET /api/v1/repos/{repo}/readme?ref=`

README 탐색(`README.md` → `README.rst` → `README.txt` → `README` 순) 후
`{ "path": "...", "format": "markdown|rst|plain", "content": "..." }`.
렌더링(HTML 변환)은 프론트엔드 책임. `markdown`만 렌더링하고 `rst`/`plain`은 평문 표시한다 (DECISIONS.md #11).

### `GET /api/v1/repos/{repo}/blame/{ref}/{path...}`

라인 범위별 `{ start_line, line_count, sha, author, authored_at }` 배열. v1 범위에 포함하되 구현 순서는 마지막.

### `GET /api/v1/repos/{repo}/archive/{ref}.{format}`

`format`: `tar.gz` | `zip`. `git archive` exec로 생성해 스트리밍. `Content-Disposition` 파일명은 `{repo}-{ref}.{format}`.

### `GET /api/v1/repos/{repo}/feed.atom`

기본 브랜치 최근 커밋의 Atom 피드 (`application/atom+xml`).

## Smart HTTP (clone/fetch 전용)

API prefix 밖, 저장소 경로 직접 매핑:

- `GET /{repo}.git/info/refs?service=git-upload-pack`
- `POST /{repo}.git/git-upload-pack`

`git upload-pack --stateless-rpc` spawn으로 처리 (advertise 시 `--advertise-refs`).
`git-receive-pack` 관련 요청은 일괄 403 `read_only`.
