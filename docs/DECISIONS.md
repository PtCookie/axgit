# 결정 이력 (ADR-lite)

번호순 누적 기록. 뒤집을 때는 삭제하지 말고 "Superseded by #N"을 남긴다.

## #1 Cgit을 자체 프론트엔드로 대체

읽기 전용 웹 UI + Smart HTTP clone이라는 두 역할을 새 스택으로 재구현한다.
push는 기존 git-server(SSH)가 계속 담당하므로 웹에는 인증·쓰기 경로가 없다.

## #2 백엔드: Rust + git2(libgit2) + axum

- Go/Rust 성능 차이는 이 규모에서 사실상 없음 — 병목은 git 객체 접근 방식과 캐싱.
- Rust 선택은 선호/학습 목적. git2는 안정적, gitoxide는 API 유동적이라 배제.
- 웹 프레임워크는 axum: tokio 생태계 표준, tower 미들웨어로 캐시/압축 처리 용이.
- 무거운 연산(archive, upload-pack)은 git 바이너리 exec (Gitea식 하이브리드).

## #3 프론트엔드: Astro(static) + React + shadcn/ui + Tailwind

SSR adapter를 쓰지 않는다. 동적 데이터는 React island의 client-side fetch.
SSR이 필요해지면 별도 결정으로 뒤집는다 (배포 컨테이너 수가 바뀌는 결정).

## #4 모노레포 + 단일 컨테이너

- web/api는 API 계약으로 강결합 → 한 저장소에서 원자적 커밋으로 관리.
- Rust 바이너리가 정적 파일 + API + Smart HTTP를 모두 서빙 → 컨테이너 1개.
  nginx/fcgiwrap 제거. git-compose에는 submodule 하나로 추가.

## #5 기존 데이터 소스와의 호환 유지

- 메타데이터: repo config `[cgit]` 섹션을 계속 읽는다 (`[axgit]` 섹션이 있으면 우선).
  git-server의 `git-init`/`post-receive`를 수정하지 않기 위함.
- 최근 활동: agefile `info/web/last-modified` → 없으면 HEAD authordate.

## #6 캐싱: cgit TTL 방식 + 검증자 개선

응답 캐시 키 `(repo, endpoint, params)`, 검증자 `HEAD sha + agefile mtime`.
sha 고정 응답은 불변 취급. 클라이언트는 ETag/immutable. (ARCHITECTURE.md#caching)

구현 시 확정 (moka 도입):

- 검증자는 git2 open 후 `head().target()` + agefile의 **raw `SystemTime`** 비교
  (`repo/meta.rs::Validator`). `.git/HEAD`/`packed-refs` 직접 파싱은 symbolic ref 처리 때문에
  깨지기 쉬워 채택하지 않음 — open 비용은 낮다.
- 캐시 계층은 tower 미들웨어가 아니라 **핸들러 공통 헬퍼**(`handlers/mod.rs::cached_response`).
  immutable 여부가 ref 해석 후에야 결정되고, params 정규화/content-type이 엔드포인트마다
  다르며, 에러 응답은 캐시하면 안 되기 때문.
- ETag는 검증자에서 생성하는 strong ETag(값 형식은 계약 아님). 요청 병합(`get_with`)은
  쓰지 않는다 — 검증자 무효화 흐름과 맞지 않고 이 규모에서 중복 계산은 허용 가능.
- **TTL(기본 300s)은 안전망**: 검증자가 못 보는 out-of-band 변경(config 수동 편집, 훅 없는
  비-HEAD push)의 staleness 상한. 엔트리당 본문 상한 1 MiB, 전체 용량은 바이트 단위(기본 32 MiB).
- 예외: repos 목록은 `ScanCache`(TTL) 유지 + 본문 해시 ETag, raw는 캐시 제외(대용량 바이너리),
  archive는 스트리밍이라 캐시 불가 + weak ETag.

## #7 도구 체인

- 패키지 매니저: **pnpm** (workspace). 루트 `pnpm-workspace.yaml`이 `web`을 패키지로 묶고,
  JS 명령은 루트에서 `pnpm --filter web <script>`로 실행한다. api는 Rust이므로 workspace 밖.
- git hooks: **lefthook** — 단일 바이너리로 Rust/JS 훅을 한 설정에서 관리 (husky+lint-staged 대체).
- 테스트: web은 **vitest**(browser mode, provider `@vitest/browser-playwright` +
  `vitest-browser-react`) + Playwright e2e, api는 cargo test + fixture repo 통합 테스트.
- 커밋: Conventional Commits (영어).

## #8 API 스타일

- REST JSON, base `/api/v1`. 계약의 규범 문서는 `docs/API.md`이며, 기계 판독용 OpenAPI 스펙은
  코드에서 생성한다 (#15).
- 커밋 작성자 이메일은 노출하지 않고 아바타 seed용 해시만 제공.
- 페이지네이션은 커밋 sha cursor 방식.

## #9 v1 범위

포함: 저장소 목록/summary/refs/log/tree/blob/raw/README/commit/diff/archive/Atom feed/Smart HTTP clone.
포함하되 마지막 순서: blame. 제외(추후): stats(커밋 통계 그래프), 저장소 검색, HTTP push.

## #10 TLS는 별도 reverse proxy 컨테이너에 위임

AXGIT 컨테이너는 HTTP만 서빙한다. git-compose 스택에 reverse proxy 서비스(nginx 등)를
별개로 추가하고 certbot 볼륨은 그쪽에 마운트한다. Rust 서버에 TLS를 넣지 않는다.

## #11 Cgit 필터 대체 (프론트엔드 렌더링)

Astro 내장 Shiki/markdown은 빌드 타임 전용이므로 런타임 데이터는 클라이언트 라이브러리로 처리:

- 문법 강조: Shiki client-side (grammar lazy-load, 대용량 파일은 생략).
- Markdown README: react-markdown + remark-gfm + rehype-sanitize (sanitize 필수 — repo 내용은 비신뢰 입력).
- reStructuredText/man: 렌더링하지 않고 평문(`<pre>`) 표시. JS 렌더러 부재, docutils급 의존성 재도입 거부.
- 커밋 메시지 링크화: React에서 정규식 linkify.
- 아바타: Gravatar 대신 이메일 해시를 seed로 한 로컬 생성 아바타 (DiceBear 권장, 외부 요청 없음).

## #12 archive/feed 구현 방식

- archive는 **`git archive` exec** (#2 하이브리드 방침). ref를 서버에서 해석한 뒤 커맨드라인에는
  **full sha만** 전달한다 — 사용자 입력이 exec 인자에 닿지 않아 인젝션 여지가 없다.
  stdout은 `tokio-util` `ReaderStream`으로 chunked 스트리밍, 별도 task가 stderr 수집 + wait로 zombie를 방지한다.
- Atom feed의 XML은 **수동 문자열 생성** (escape 헬퍼 포함). 고정 구조의 평평한 문서 하나에
  런타임 XML crate를 들이지 않는다. quick-xml은 dev-dependency로만 (테스트의 well-formed 검증).
- base URL 설정을 추가하지 않는다. feed의 절대 URL은 `X-Forwarded-Proto`/`X-Forwarded-Host`/`Host`
  헤더에서 재구성한다 (#10 reverse proxy 방침의 귀결). entry `<id>`는 host와 무관하게 영구
  안정적이어야 피드 리더가 중복 표시하지 않으므로 `urn:sha1:{sha}` 형식을 쓴다.

## #13 Smart HTTP: `git upload-pack --stateless-rpc` 직접 spawn

- `git http-backend`(CGI) 대신 **upload-pack 직접 spawn** (#2 하이브리드 방침, archive의
  tokio::process + `ReaderStream` + reaper 패턴 재사용). CGI env 조립이 필요 없고, 프로세스
  자체가 upload-pack으로 고정되어 receive-pack이 **구조적으로** 도달 불가하다 (읽기 전용
  invariant를 코드 리뷰가 아니라 구조가 보장).
- 커맨드라인에는 `open_named`가 검증·해석한 git dir 경로만 전달한다 (사용자 입력이 exec
  인자에 닿지 않음 — #12와 동일 원칙). protocol v2는 `Git-Protocol` 요청 헤더를 위생 검사 후
  `GIT_PROTOCOL` env로 전달해 지원.
- gzip 요청 body는 **전체 버퍼링 + flate2 동기 해제** (spawn_blocking 안). negotiation
  데이터는 거대 repo에서도 수 MB 수준이라 스트리밍 gunzip(async-compression)의 복잡도가
  불필요하다. 상한: 압축 8 MiB (DefaultBodyLimit), 해제 후 64 MiB (압축 폭탄 방어).
- stdin 쓰기는 별도 task로 분리해 write/read 데드락 가능성을 차단, stdout은 스트리밍.
- moka 응답 캐시 + ETag 일괄 도입은 이 작업에 묶지 않고 다음 작업으로 분리했다
  (Smart HTTP 응답은 no-cache라 캐시와 직교).

## #14 blame: git2 `blame_file` (exec 아님)

- ROADMAP에 "vs `git blame` exec 중 벤치 후 결정"으로 유예했던 항목을 **git2 `blame_file`**로
  확정. 근거: (1) ARCHITECTURE.md가 이미 blame을 git2 담당 목록에 명시, (2) exec 방식과 달리
  경로가 커맨드라인 인자로 전혀 노출되지 않는다(archive/Smart HTTP는 인젝션 차단을 위해
  "exec에는 해석된 sha만" 원칙을 지키는데, blame은 애초에 exec가 아니므로 그 제약 자체가
  불필요), (3) `git blame --line-porcelain` 출력 파서를 새로 작성하는 비용 대비 git2 API가
  hunk 단위 구조체를 직접 제공. 별도 벤치마크는 하지 않았다 — 대형 히스토리에서 libgit2가
  느리다고 확인되면 (log의 path 필터와 동일한 우려) exec fallback을 그때 재검토한다.
- 바이너리/1 MiB 초과 판정은 blob 엔드포인트와 동일 상한을 재사용 (`blob::BLOB_CONTENT_LIMIT`,
  `classify` 헬퍼로 공유) — 사용자 입장에서 "이 파일은 볼 수 없다"는 판단 기준이 blob/blame
  간에 갈릴 이유가 없다.
- range마다 커밋 `summary`를 포함시킨다 (커밋을 이미 조회하므로 추가 비용이 거의 없고,
  blame 거터를 그리는 프론트가 커밋 상세 API를 range 개수만큼 부르는 걸 막는다).
- rename 추적(`git blame --follow` 상당)은 도입하지 않는다 — libgit2 기본 옵션 그대로
  파일 내 이동만 반영. 커밋 상세/diff의 rename 감지(#8 이전부터의 기존 방침)와는 별개 결정.

## #15 OpenAPI 스펙은 utoipa로 코드에서 생성

- #8에서 "추후 검토"로 미뤄둔 OpenAPI를 web 구현 **직전에** 도입한다. ROADMAP의 web 절이 예고했던
  "`types.ts` 수동 정의 + API.md와 동기화" 부채를 애초에 만들지 않기 위해서다.
- **손으로 쓴 `openapi.yaml`은 거부.** 코드와 별개의 두 번째 진실 공급원이 되어 드리프트한다
  (실제로 산문 문서인 API.md조차 이미 두 군데 — 사라진 `501 not_implemented`, 실제와 다른
  `charset=utf-8` — 드리프트해 있었다). 대신 `#[utoipa::path]` + `#[derive(ToSchema)]`로 코드에서
  생성하고 결과를 `docs/openapi.json`에 커밋한다. `api/tests/openapi_test.rs`가 스냅샷 일치와
  "라우팅된 오퍼레이션 = 스펙의 오퍼레이션"을 검증하므로, 어노테이션을 빠뜨린 엔드포인트는 CI에서 걸린다.
  갱신은 `AXGIT_UPDATE_OPENAPI=1 cargo test --test openapi_test` (전용 바이너리 추가 없이).
- **`utoipa-axum`의 `OpenApiRouter` 자동 수집은 쓰지 않는다.** tree/blob/raw/blame/archive는 axum에서
  `{*rest}` 단일 catch-all이고 ref/path 경계는 요청 시점에 최장 매칭으로 정해지므로, 자동 수집하면
  `/tree/{rest}`가 문서화되어 API.md의 `/tree/{ref}/{path}` 계약과 어긋난다. 경로는 `api/src/openapi.rs`에
  손으로 적는다 — 라우터와 이중 관리가 되지만, 위 오퍼레이션 목록 테스트가 그 비용을 감당한다.
- **Swagger UI는 `utoipa-swagger-ui`의 `vendored` 기능**으로 서빙한다 (`/swagger-ui`). 에셋이
  크레이트에 포함되어 빌드·런타임 모두 외부 요청이 없다 — 아바타를 로컬 생성으로 결정한 #11과 같은
  원칙이고, self-hosted 환경이 오프라인이어도 동작한다. 대가는 바이너리 수 MB 증가.
  읽기 전용·무인증 API라 노출에 위험이 없으므로 별도 on/off 플래그는 두지 않는다.
- **닫힌 문자열 집합은 진짜 enum으로 바꿨다**: `DiffStatus`, `EntryKind`, `LineOrigin`, `ReadmeFormat`.
  `&'static str`/`char`는 스펙에서 그냥 `string`이 되어 문서 가치도 생성 타입의 이점도 없다.
  JSON 출력은 serde rename으로 완전히 동일하게 유지된다(`LineOrigin`은 `" "`/`"+"`/`"-"`).
  같은 이유로 항상 직렬화되는 `Option` 필드에는 `#[schema(required = true)]`를 달아,
  생성 타입이 `field?: T | null`이 아니라 `field: T | null`이 되게 했다.
- 에러 body는 `serde_json::json!` 리터럴에서 `ErrorResponse`/`ErrorBody` 구조체로 바꿨다 —
  스키마를 붙일 대상이 필요했고, 그 덕에 스펙이 실제 응답에서 벗어날 수 없다.
- web은 `docs/openapi.json`에서 `openapi-typescript`로 `web/src/lib/api/types.ts`를 생성한다
  (`pnpm gen:types`). 생성 파일이므로 직접 수정하지 않고 eslint 대상에서도 제외한다.

## #16 정적 빌드 + SPA fallback

- Astro static 모드에서는 `{repo}`가 URL 첫 세그먼트라 빌드 타임에 저장소별 라우트를 열거할 수
  없다 (`getStaticPaths`로 만들려면 빌드 시점에 저장소 목록이 필요한데, 그건 배포 환경마다 다르다).
  `/{repo}/tree/...` 같은 저장소 하위 경로는 클라이언트 라우팅으로 처리하고, api는 해당 경로 요청에
  `index.html`을 fallback으로 서빙해 SPA 셸을 띄운다.
- **주의**: axum에서 `nest("/api/v1")`/merge된 라우트는 바깥 `fallback_service`를 상속한다. 배선 시
  api 라우터에 JSON 404 fallback을 명시해야 `/api/v1/bogus`가 (SPA용 HTML 셸이 아니라) 정상적인
  JSON 에러를 돌려준다 (`api/src/routes.rs:60`).
- 이번 커밋(저장소 목록 페이지)은 방침만 기록한다. 실제 api 배선과 클라이언트 라우터 도입은
  저장소별 페이지 구현 커밋에서 함께 한다.
