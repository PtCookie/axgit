# Axgit 구현 로드맵

`api/`의 엔드포인트를 어떤 순서로, 어떤 설계로 구현할지 세션 간에 이어가기 위한 문서.
완료 시 다음 절을 갱신하고, "다음 구현" 절을 그 다음 대상으로 교체할 것.

## 완료됨

- `GET /api/v1/repos` — 저장소 스캔(`api/src/repo/scan.rs`, `meta.rs`), TTL 캐시(`cache.rs`),
  `[axgit]`/`[cgit]` 메타데이터 읽기. 나머지 엔드포인트는 501 stub, `git-receive-pack`은 403.
- `GET /api/v1/repos/{repo}`, `GET /api/v1/repos/{repo}/refs` — 저장소 요약 + refs.
  이때 확정된 공통 기반: `repo/open.rs::open_named`(`{repo}` 검증 + bare open, 이후 모든
  per-repo 엔드포인트가 사용할 것), `repo/refs.rs::list_refs`, `meta.rs`의
  `git_time_to_zoned`/`format_rfc3339` 공유. 핸들러의 git2 작업은 `spawn_blocking`으로 감싼다
  (이후 엔드포인트도 동일하게). ETag/Cache-Control은 유예(아래 캐시 항목 참고).
- `GET /api/v1/repos/{repo}/commits` — 커밋 로그 (`repo/commits.rs::log`). 확정된 설계:
  - **ref 해석 공통 헬퍼 `repo/resolve.rs::resolve_commit`** (브랜치/태그/sha → Commit,
    실패는 전부 `RefNotFound`). 이후 tree/blob/raw가 재사용할 것.
    git2 에러를 bare `?`로 올리면 500이 되므로 사용자 입력 유래 호출은 반드시 `map_err`.
  - cursor는 inclusive이며 `ref`를 무시. 잘못된 cursor는 400 (opaque 토큰). `limit` 초과는
    400 (clamp 안 함, API.md 명시).
  - `path` 필터는 git2 tree entry-id 비교 (`git log` exec 아님). merge는 모든 부모와 다를 때만
    포함 — git log simplification의 근사. 긴 히스토리에서 드물게 변경되는 경로는 walk가 길어질
    수 있음: 성능 문제가 생기면 `git log` exec fallback으로 전환 검토.
  - `email_hash` = sha256(trim + lowercase). 테스트 헬퍼 `tests/common::commit_history`
    (다중 커밋 히스토리, 커밋별 고정 날짜) 신설.
- `GET /api/v1/repos/{repo}/commits/{sha}`, `GET /.../commits/{sha}/diff?path=` — 커밋 상세 +
  구조화 diff (`repo/diff.rs`, `handlers/commits.rs` 신설 — commits 관련 핸들러는 `list_commits`
  포함 이쪽으로 이동, 이후 엔드포인트도 리소스별 핸들러 파일로 분할). 확정된 설계:
  - diff는 git2 `diff_tree_to_tree` + **첫 부모 기준** (merge 포함, root는 empty tree).
    rename 감지는 `find_similar` libgit2 기본값. exec fallback은 병목 확인 시로 유예.
  - 상한: 파일당 렌더 1000줄(hunk 단위 절단), diff 응답 파일 300개. 초과 시 `truncated: true`,
    `additions`/`deletions`는 항상 전체 값. 바이너리는 `binary: true` + `hunks: []`.
  - `?path=`는 리터럴 pathspec, 미존재/미변경 경로는 `files: []` (log의 빈 결과 선례 유지;
    `PathNotFound`는 tree/blob용으로 남김).
  - **immutable Cache-Control 선반영**: 요청의 `{sha}`가 해석된 full sha와 문자열 일치할 때만
    부착 (`handlers/commits.rs::sha_addressed_json`). ETag/응답 캐시 일괄 도입은 여전히 유예.
  - 테스트 헬퍼 `tests/common::commit_all`(다중 파일/삭제/rename/바이너리 커밋 구성),
    `get_json_with_headers` 신설.

- `GET /api/v1/repos/{repo}/tree|blob|raw/{ref}/{path...}`, `GET /.../readme?ref=` —
  파일 브라우징 (`repo/tree.rs`, `blob.rs`, `readme.rs`, `handlers/files.rs`). 확정된 설계:
  - **`{ref}/{path...}` 분리는 refs 최장 매칭** (`repo/resolve.rs::resolve_ref_path`):
    브랜치/태그명과 일치하는 최장 선행 세그먼트 열이 ref (git ref 규칙상 유일), 없으면
    첫 세그먼트를 ref(sha)로 fallback. path의 `.`/`..`/빈 세그먼트는 400.
  - blob 바이너리 판정은 `Blob::is_binary()` + 비UTF-8도 바이너리 취급. **content 상한
    1 MiB** (`repo/blob.rs::BLOB_CONTENT_LIMIT`, readme에도 적용). raw는 상한 없음,
    MIME은 mime_guess 확장자 기반 + 내용 fallback, `nosniff` 부착.
  - readme는 대소문자 무시 우선순위 탐색, symlink·바이너리·초과 후보는 skip, 없으면 404.
  - `sha_addressed_json`은 `handlers/mod.rs`로 이동해 공유 (commits와 files 공용).
    tree entry size는 `odb().read_header()` (내용 로드 없음).

- `GET /api/v1/repos/{repo}/archive/{ref}.{format}`, `GET /.../feed.atom` —
  아카이브 스트리밍 + Atom 피드 (`handlers/archive.rs`, `handlers/feed.rs`, DECISIONS #12). 확정된 설계:
  - archive는 `git archive` exec: ref 해석 후 **exec에는 full sha만 전달** (인젝션 차단),
    stdout을 `tokio-util` `ReaderStream`으로 스트리밍, reaper task가 stderr 수집 + zombie 방지.
    `{ref}.{format}` 파싱은 접미사 매칭 (`.tar.gz` 우선 → `.zip`, 그 외 400).
    파일명/`--prefix`는 `[A-Za-z0-9._-]` 외 문자를 `-` 치환한 `{repo}-{safe_ref}`.
    full sha 요청만 immutable Cache-Control (기존 선례).
  - feed는 `commits.rs::log` 재사용 (HEAD, 20개 고정), XML은 수동 문자열 + escape 헬퍼
    (quick-xml은 dev-dep 검증 전용). base URL은 `X-Forwarded-*`/`Host` 헤더에서 재구성,
    entry id는 `urn:sha1:{sha}`. 빈 저장소는 200 + entry 0개.
  - 테스트 헬퍼 `tests/common::get_bytes_with_request_headers` (요청 헤더 주입) 신설.
    tar.gz는 실제 `tar -xzf`로 풀어 검증, zip은 `git archive` 직접 실행 출력과 바이트 비교.

## 다음 구현: Smart HTTP upload-pack

### Context

DECISIONS.md #9 순서의 다음 단계 (blame은 마지막 고정).
범위: `GET /{repo}.git/info/refs?service=git-upload-pack`, `POST /{repo}.git/git-upload-pack`
(`api/src/smart_http.rs`에 설계 요약만 있음, 현재 501/403 stub. `docs/API.md` Smart HTTP 절 참고).

### 착수 시 검토할 것 (전 세션에서 확정하지 않음 — 착수 시점에 설계할 것)

- `git upload-pack --stateless-rpc` spawn (advertise 시 `--advertise-refs`) vs
  `git http-backend` CGI 방식 — archive에서 확립한 tokio::process + ReaderStream 패턴 재사용.
- 요청 body의 gzip 해제 (`Content-Encoding: gzip`), pkt-line 서비스 헤더, no-cache 헤더 규약.
- 통합 테스트는 실제 `git clone http://…` round-trip (docs/ARCHITECTURE.md 테스트 전략).
- 이 시점부터 moka 기반 (repo, endpoint, params) 응답 캐시 도입 검토
  (`api/src/cache.rs` 상단 주석 및 `docs/ARCHITECTURE.md#caching` 참고).
  **API.md의 ETag/Cache-Control 규약도 아직 미구현** — 검증자(HEAD sha)를 쓰는
  응답 캐시와 함께 이 시점에 일괄 구현한다 (summary/refs 구현 시 의도적으로 유예).

## 이후 항목 (DECISIONS.md #9 순서, 착수 전 재검토 필요)
- blame (v1 포함, 구현 순서는 마지막 — API.md 명시).
