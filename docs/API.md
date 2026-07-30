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
  full sha와 정확히 일치할 때만** 이 헤더가 붙는다. 이 응답에는 `ETag`가 없다.
- 그 외: `ETag` + `Cache-Control: no-cache`. `If-None-Match`가 일치하면 **304**(본문 없음,
  `ETag`/`Cache-Control` 동반). ETag 값은 **opaque**하며 형식은 계약이 아니다 — 서버는 해당
  repo의 HEAD sha + agefile mtime에서 생성하므로 push 직후 값이 바뀐다.
  비교는 RFC 9110의 weak comparison(`W/` 접두 무시).
- 예외:
  - `GET /api/v1/repos`(목록)는 특정 repo에 종속되지 않으므로 ETag가 **응답 본문 해시**다.
  - `raw`는 서버 응답 캐시 대상이 아니지만 비-sha 요청에 ETag가 붙는다 (304로 전송량만 절약).
  - `archive`는 **weak ETag**(`W/"..."`). 일치하면 `git archive` 실행 없이 304.
  - Smart HTTP 엔드포인트는 항상 `no-cache`이며 ETag를 쓰지 않는다.

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

디렉토리 목록. `{ref}`는 `/`를 포함할 수 있으므로(브랜치/태그명) 경로와의 경계는
**refs 최장 매칭**으로 해석한다: 선행 세그먼트 열이 기존 브랜치/태그명과 일치하는 가장 긴
prefix가 ref (git ref 규칙상 `a`와 `a/b`는 공존 불가 → 유일), 일치가 없으면 첫 세그먼트를
ref(커밋 sha 등)로 간주한다. blob/raw도 동일 규칙. `{path...}` 생략 시 루트 tree.

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

- `type`: `tree` | `blob` | `symlink`(mode 120000) | `commit`(submodule gitlink).
- `mode`: 6자리 8진수 문자열. `size`: blob만, 그 외 `null`.
- 정렬: tree 우선, 이후 이름 오름차순.
- 경로가 없거나 디렉토리가 아니면 `404 path_not_found`.
  path의 `.`/`..`/빈 세그먼트는 `400 invalid_param`.
- 요청의 `{ref}`가 해석된 full sha와 문자열 일치하면 immutable Cache-Control
  (캐싱 헤더 절 참고 — blob/raw/readme도 동일).

### `GET /api/v1/repos/{repo}/blob/{ref}/{path...}`

파일 메타 + 내용.

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

- `content`: UTF-8 텍스트. 바이너리(libgit2 NUL 휴리스틱 또는 비UTF-8)는
  `binary: true` + `content: null` (raw로 유도).
- **1 MiB 초과는 `too_large: true` + `content: null`** (`size`는 항상 전체 값).
- symlink는 mode `120000`에 `content` = 링크 대상 경로.
- 경로가 없거나 파일이 아니면(디렉토리·submodule) `404 path_not_found`.

### `GET /api/v1/repos/{repo}/raw/{ref}/{path...}`

파일 원문 스트리밍. cgit plain view에 해당. 크기 상한 없음.

- `Content-Type`: 확장자 기반 감지(mime_guess). 미감지 시 텍스트는
  `text/plain; charset=utf-8`, 바이너리는 `application/octet-stream`.
- 저장소 내용은 비신뢰 입력이므로 항상 `X-Content-Type-Options: nosniff`를 부착한다.

### `GET /api/v1/repos/{repo}/readme?ref=`

README 탐색 후 `{ "path": "...", "format": "markdown|rst|plain", "content": "..." }`.
렌더링(HTML 변환)은 프론트엔드 책임. `markdown`만 렌더링하고 `rst`/`plain`은 평문 표시한다 (DECISIONS.md #11).

- 루트 tree에서 `README.md` → `README.rst` → `README.txt` → `README` 순,
  **대소문자 무시** 매칭. symlink·바이너리·1 MiB 초과 후보는 건너뛴다.
- `path`는 tree상의 실제 파일명(대소문자 유지). 못 찾으면 `404 path_not_found`.
- `ref` 생략 시 HEAD. 빈 저장소(unborn HEAD)·해석 실패는 `404 ref_not_found`.
- `content`에는 blob의 1 MiB 상한이 동일하게 적용된다.

### `GET /api/v1/repos/{repo}/blame/{ref}/{path...}`

라인 범위별 `{ start_line, line_count, sha, author, authored_at }` 배열. v1 범위에 포함하되 구현 순서는 마지막.

### `GET /api/v1/repos/{repo}/archive/{ref}.{format}`

`format`: `tar.gz` | `zip`. `git archive` exec로 생성해 chunked 스트리밍 (Content-Length 없음).
ref 해석 후 **exec에는 full sha만 전달**한다 (사용자 입력이 커맨드라인에 닿지 않음).

- `Content-Type`: `application/gzip` | `application/zip`. 항상 `X-Content-Type-Options: nosniff`.
- `Content-Disposition: attachment; filename="{repo}-{safe_ref}.{format}"`.
  아카이브 내부 루트 디렉토리(`--prefix`)도 동일하게 `{repo}-{safe_ref}/`.
  `safe_ref`는 ref의 `[A-Za-z0-9._-]` 외 문자를 전부 `-`로 치환한 값 (예: `feature/x` → `feature-x`).
- `{ref}.{format}` 파싱은 접미사 매칭: `.tar.gz` 우선, 다음 `.zip` (ref 자체의 `.`/`/`와 충돌 없음.
  `.zip`으로 끝나는 브랜치명은 zip 요청으로 해석된다). 그 외 접미사는 `400 invalid_param`.
- 요청의 `{ref}`가 해석된 full sha와 문자열 일치하면 immutable Cache-Control, 그 외에는
  weak ETag + `no-cache` (캐싱 헤더 절). `If-None-Match` 일치 시 git 프로세스를 띄우지 않고 304.
- 스트리밍 개시 후 git 프로세스가 실패하면 상태코드를 바꿀 수 없으므로 응답이 중간에서
  끊긴다 (클라이언트는 다운로드 실패로 인지, 서버는 stderr를 로그에 남김).

### `GET /api/v1/repos/{repo}/feed.atom`

기본 브랜치(HEAD) 최근 커밋 **20개**의 Atom 피드. `Content-Type: application/atom+xml; charset=utf-8`.

- feed: `<title>` = repo 이름, `<subtitle>` = description (있을 때만), `<id>`와 `rel="self"` link =
  이 엔드포인트의 절대 URL, `<updated>` = 최신 커밋 authordate (커밋이 없으면 epoch).
- entry: `<title>` = 커밋 summary (non-utf8이면 `(no message)`), `<id>` = **`urn:sha1:{full sha}`**
  (host와 무관하게 안정 — 피드 리더의 중복 방지), `<updated>` = authordate,
  `<author><name>`만 (이메일은 해시조차 미포함), `rel="alternate"` link = 커밋 상세 API URL
  (**잠정** — web UI 커밋 페이지 라우트 확정 시 그쪽으로 교체).
- 절대 URL의 base는 `X-Forwarded-Proto`(기본 `http`) + `X-Forwarded-Host` → `Host`(기본
  `localhost`) 헤더에서 재구성한다 (별도 base URL 설정 없음, DECISIONS.md #12).
- 빈 저장소(unborn HEAD)는 404가 아니라 entry 없는 피드로 `200`.
- ETag + `Cache-Control: no-cache` (캐싱 헤더 절). 본문에 base URL이 들어가므로 서버 응답
  캐시 키에도 base URL이 포함된다.

## Smart HTTP (clone/fetch 전용)

API prefix 밖, 저장소 경로 직접 매핑. `git upload-pack --stateless-rpc` spawn으로 처리한다
(advertise 시 `--advertise-refs`, DECISIONS.md #13). smart 프로토콜 전용 — dumb 프로토콜
(`service` 파라미터 없는 info/refs)은 지원하지 않는다.

### `GET /{repo}.git/info/refs?service=git-upload-pack`

- `200` 응답: `Content-Type: application/x-git-upload-pack-advertisement`,
  `Cache-Control: no-cache`. body는 pkt-line 서비스 헤더
  `001e# service=git-upload-pack\n0000` 뒤에 upload-pack의 ref advertisement.
- 클라이언트의 `Git-Protocol` 요청 헤더(`version=2` 등)는 `GIT_PROTOCOL` env로 upload-pack에
  전달된다 — protocol v2 협상 지원. v2에서도 서비스 헤더 pkt-line은 동일하게 붙는다.

### `POST /{repo}.git/git-upload-pack`

- 요청 body는 upload-pack negotiation 데이터 (`application/x-git-upload-pack-request` —
  Content-Type은 검증하지 않음). `Content-Encoding: gzip`이면 서버가 해제한다.
  body 상한: 압축 상태 8 MiB (`413`), 해제 후 64 MiB (`400`).
- `200` 응답: `Content-Type: application/x-git-upload-pack-result`,
  `Cache-Control: no-cache`, pack 데이터 chunked 스트리밍. 스트리밍 시작 후 upload-pack이
  비정상 종료하면 상태코드 변경 없이 스트림이 짧게 끊긴다 (클라이언트는 early EOF).

### 상태코드

| 상황 | 상태 | code |
| --- | --- | --- |
| 정상 | `200` | — |
| `service` 누락/미지원, gzip 해제 실패, 해제 후 상한 초과 | `400` | `invalid_param` |
| `git-receive-pack` 관련 요청 일체 (info/refs의 service 포함) | `403` | `read_only` |
| 저장소 없음, `.git` 접미사 없는 경로 | `404` | `repo_not_found` |
| 압축 body 상한 초과 | `413` | — (axum 기본 응답) |
| upload-pack spawn/advertise 실패 | `500` | `internal` |

에러 body는 다른 엔드포인트와 같은 JSON 형식이다 (git 클라이언트는 무시하고 상태코드만 표시).
