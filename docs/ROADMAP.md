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

- `GET /{repo}.git/info/refs?service=git-upload-pack`, `POST /{repo}.git/git-upload-pack` —
  Smart HTTP upload-pack (`api/src/smart_http.rs`, DECISIONS #13). 확정된 설계:
  - `git upload-pack --stateless-rpc` 직접 spawn (advertise 시 `--advertise-refs`,
    http-backend CGI 아님) — archive의 exec/ReaderStream/reaper 패턴 재사용. exec에는
    `open_named`가 해석한 git dir만 전달. receive-pack 403 배선은 기존 그대로.
  - protocol v2는 `Git-Protocol` 헤더를 위생 검사 후 `GIT_PROTOCOL` env로 전달.
    v2에서도 advertisement의 pkt-line 서비스 헤더는 동일하게 prepend.
  - gzip 요청 body는 flate2 전체 버퍼링 해제 (압축 8 MiB / 해제 후 64 MiB 상한).
    stdin 쓰기는 별도 task (데드락 방어). service 누락/미지원은 400 (dumb 미지원).
  - 테스트: oneshot 프로토콜 검증 + 실제 리스너(`tests/common::serve`)로
    `git clone`/`--depth 1`/`fetch`/push 거부 round-trip (multi_thread flavor 필수 —
    git CLI가 테스트 스레드를 블로킹).

- **moka 응답 캐시 + ETag/Cache-Control 일괄 도입** (`api/src/cache.rs`, `handlers/mod.rs`,
  DECISIONS #6). summary/refs 시점부터 유예해 온 부채를 해소. 확정된 설계:
  - 검증자 `repo/meta.rs::Validator` = HEAD sha + agefile **raw `SystemTime`**. git2 open 후
    `head().target()`으로 읽는다 (HEAD 파일 직접 파싱 아님). 검증자 조회도 spawn_blocking.
  - **캐시 계층은 핸들러 공통 헬퍼 `handlers/mod.rs::cached_response`** (tower 미들웨어 아님).
    `compute` 클로저가 `(immutable, 직렬화된 body)`를 반환하고, 미스 경로는 open 1회로
    검증자와 본문을 같은 스냅샷에서 얻는다. 에러는 캐시하지 않고 기존 엔트리를 폐기.
    기존 `sha_addressed_json`은 전 핸들러가 이 헬퍼로 옮겨가며 제거.
  - immutable 엔트리는 `validator: None`으로 저장 → 히트 시 **저장소를 열지 않는다**
    (유일한 zero-git2 경로). 그 외는 ETag(검증자 기반 strong) + `no-cache` + 304.
  - 예외: repos 목록은 ScanCache 유지 + 본문 sha256 ETag, raw는 캐시 제외(ETag만),
    archive는 스트리밍이라 캐시 불가 + weak ETag(일치 시 exec 생략), feed는 base URL을
    캐시 키에 포함. Smart HTTP는 불변.
  - moka 설정: 바이트 weigher + `AXGIT_CACHE_RESPONSE_MAX_BYTES`(32 MiB),
    엔트리당 본문 1 MiB 상한, TTL(`AXGIT_CACHE_RESPONSE_TTL` 300s)은 out-of-band 변경 안전망.
    `get_with`(요청 병합)는 검증자 흐름과 안 맞아 미사용.
  - 테스트: `tests/cache_test.rs` 신설 (라우터 clone으로 상태 공유 — 히트/무효화/304/
    immutable 엔트리 검증). Config 필드 추가 비용은 선행 커밋의
    `tests/common::test_config`/`router_for` 중앙화로 흡수.
  - 후속 검토: cursor·full-sha `ref`로 조회한 commits 페이지도 사실상 불변이므로 immutable
    승격 여지가 있으나, "sha가 URL 경로에 포함"이라는 API.md 계약을 바꾸게 되어 보류.

- `GET /api/v1/repos/{repo}/blame/{ref}/{path...}` — 라인 범위별 attribution
  (`repo/blame.rs`, `handlers/files.rs::get_blame`). v1 범위의 마지막 엔드포인트
  (DECISIONS #9, #14). 확정된 설계:
  - **git2 `Repository::blame_file`** 채택 (exec 아님) — 경로가 커맨드라인에 닿지 않고,
    ARCHITECTURE.md가 이미 blame을 git2 담당으로 명시. 벤치 없이 채택; 대형 히스토리에서
    병목이 확인되면 `git blame --line-porcelain` exec fallback을 후속 검토.
  - `{ref}/{path...}` 분리는 기존 `repo/resolve.rs::resolve_ref_path` 재사용.
  - 바이너리/1 MiB 초과는 `blob::classify`(blob 판정 로직을 `blob_at`/`classify`로 추출해 공유)로
    걸러 `ranges: []` + `lines: 0`. 빈 파일도 동일 — libgit2가 반환하는 0-line 훙크를 걸러낸다.
  - hunk마다 `final_commit_id()`로 커밋을 1회만 조회해 `summary`/`author`/`authored_at` 캐시
    (`HashMap<Oid, _>`) — 같은 커밋이 여러 range에 등장해도 재조회하지 않는다. `CommitAuthor`에
    `Clone` 추가, `commits.rs::signature_info`/`time_rfc3339` 공개해 재사용.
  - 캐싱은 tree/blob과 동일하게 `cached_response`로 (sha-addressed면 immutable).
  - 후속 검토: `git blame --follow`(rename 추적)는 도입하지 않음 — 필요해지면 exec fallback과
    함께 재검토.

- **OpenAPI 스펙 + Swagger UI + web 타입 생성** (`api/src/openapi.rs`, `docs/openapi.json`,
  `web/src/lib/api/types.ts`, DECISIONS #15). web 구현 전에 계약을 기계 판독 가능하게 만들어
  "types.ts 수동 정의" 부채를 없앴다. 확정된 설계:
  - utoipa 5 + utoipa-swagger-ui 9(`vendored`). 스펙은 `/api/v1/openapi.json`, UI는 `/swagger-ui`.
    `utoipa-axum` 자동 수집은 **미사용** — catch-all 라우트 때문에 경로를 `openapi.rs`에 손으로 적는다.
  - `docs/openapi.json`을 커밋하고 `tests/openapi_test.rs`가 (1) 스냅샷 일치 (2) 라우팅된
    오퍼레이션 = 스펙 오퍼레이션(16개) (3) 두 라우트의 스모크를 검증. 갱신은
    `AXGIT_UPDATE_OPENAPI=1 cargo test --test openapi_test`.
  - `&'static str`/`char` 필드를 enum으로 승격 (`DiffStatus`/`EntryKind`/`LineOrigin`/`ReadmeFormat`),
    항상 직렬화되는 `Option`에는 `#[schema(required = true)]`. JSON 출력은 불변 — 기존 통합 테스트
    129개가 그대로 통과하는 것이 그 증거다. 에러 body는 `ErrorResponse`/`ErrorBody` 구조체로 타입화.
  - web은 `pnpm gen:types`(openapi-typescript + prettier)로 `types.ts` 생성. eslint 대상 제외.
  - API.md 드리프트 2건 정정: 사라진 `501 not_implemented` 행, 실제와 다른 `charset=utf-8`.

## 다음 구현: web 스캐폴딩

### Context

api의 v1 엔드포인트 표면이 완성되었고(DECISIONS #9 전체 완료), OpenAPI 스펙과 그로부터
생성한 `web/src/lib/api/types.ts`도 준비되어 있다. `web/`에는 Astro 초기 스캐폴드만 있으므로
실제 화면을 여기서부터 세운다.

### 착수 시 검토할 것

- ARCHITECTURE.md의 프론트엔드 절 그대로: Astro **static** 모드 + React islands + shadcn/ui
  + Tailwind. SSR adapter 추가 금지 (배포 형태 변경, 별도 논의 필요).
- 라우트: `/`(저장소 목록), `/{repo}/`(summary), `/{repo}/log`, `/{repo}/tree/[...path]`,
  `/{repo}/blob/[...path]`, `/{repo}/commit/{sha}`, `/{repo}/refs`, `/{repo}/blame/[...path]`.
  ref 선택은 URL 쿼리 `?ref=`로 통일.
- API 응답 타입은 `web/src/lib/api/types.ts`(생성 파일, 직접 수정 금지)에서 가져다 쓴다.
  `components["schemas"]["RepoInfo"]` 식으로 참조하거나 `web/src/lib/api/`에 얇은 alias를 둔다.
  API가 바뀌면 `pnpm gen:types`로 재생성. 첫 커밋은 fetch 클라이언트 + 저장소 목록 페이지 정도의
  최소 뼈대로 시작 검토.
- shadcn/ui 컴포넌트는 `web/src/components/ui/`에 생성(vendored, 수정 가능).
- vitest + Testing Library, fetch mocking / fixture JSON으로 테스트.
