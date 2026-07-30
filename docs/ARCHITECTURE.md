# Axgit 아키텍처

## 배경

기존 git-compose 스택의 git-web(Cgit + Nginx + fcgiwrap)을 대체한다. Cgit 분석에서 얻은 요구사항:

- Cgit은 git 내부 라이브러리를 링크한 C CGI 프로그램으로, nginx → fcgiwrap → cgit.cgi 경로로 실행되고
  디스크 캐시(`cache-root`)로 CGI 실행 비용을 상쇄한다.
- 저장소별 메타데이터는 bare repo의 `config` 안 `[cgit]` 섹션에, 최근 활동 시각은
  agefile(`info/web/last-modified`, post-receive 훅이 갱신)에 저장된다. **Axgit은 이 두 소스를 그대로 읽는다.**
- clone 트래픽은 Cgit이 아닌 `git-http-backend`가 처리했다 (upload-pack만, push는 SSH 전용).
- 대체해야 할 화면: index(저장소 목록), summary, log, tree, blob/plain, commit/diff, refs,
  blame, stats(후순위), snapshot, Atom feed.

## 전체 구성

```
브라우저 ──→ Axgit 컨테이너 (단일)
              ├─ /              → Astro 정적 빌드 결과물 (web/dist)
              ├─ /api/v1/*      → axum JSON API ──→ git2 / git exec ──→ /srv/git (ro)
              └─ /{repo}.git/*  → Smart HTTP (git upload-pack --stateless-rpc)
git push ──→ SSH 2222 → git-server 컨테이너 (기존 유지, 변경 없음)
```

- git-compose에서 기존 `git-web` 서비스를 이 이미지로 교체. `git-repository` 볼륨을 `:ro` 마운트.
- TLS는 git-compose 스택에 별도 추가하는 reverse proxy 컨테이너가 종료한다 (certbot 볼륨도 그쪽으로 이동).
  Axgit은 HTTP만 서빙한다 (DECISIONS.md #10).

## 백엔드 (api/)

### 스택

- **axum** + tokio. 프레임워크 결정 근거는 DECISIONS.md 참고.
- **git2** (libgit2 바인딩): refs, tree, blob, commit 조회, diff, blame.
- **git 바이너리 exec**: `git archive`(스냅샷), `git upload-pack --stateless-rpc`(Smart HTTP),
  대형 repo에서 병목이 확인되는 연산의 escape hatch. libgit2로 전부 해결하려 하지 않는다.

### 저장소 스캔

- 기동 시 + 주기적으로 `AXGIT_REPO_ROOT`(기본 `/srv/git`)에서 `*.git` 디렉토리 스캔 (cgit `scan-path` 대응).
- 각 repo의 config에서 `[axgit]` → `[cgit]` 순으로 `section`/`name`/`owner`/`desc` 파싱.
- 스캔 결과는 in-memory 목록으로 유지, TTL(기본 60s) 경과 시 재스캔 (cgit `cache-scanrc-ttl` 대응).

### Caching

cgit의 디스크 TTL 캐시를 개선한 2계층:

1. **서버 응답 캐시** — in-memory LRU (`moka`, `cache.rs`). 키: `(repo, endpoint, 정규화된 params)`.
   - 검증자: 해당 repo의 **HEAD sha + agefile mtime** (`repo/meta.rs::Validator`).
     캐시 히트 시 검증자가 다르면 폐기 → cgit의 순수 TTL과 달리 push 직후 즉시 반영되고,
     조용한 repo는 TTL까지 재사용 가능.
   - 검증자 조회 자체(HEAD 읽기 + agefile stat)는 저렴하므로 매 요청 수행.
   - 커밋 sha가 키에 포함된 응답(commit 상세, diff, sha 기준 tree/blob)은 불변이므로 검증 없이 LRU만
     — 이 경로만 캐시 히트 시 저장소를 아예 열지 않는다.
   - 용량은 본문 바이트 기준(`AXGIT_CACHE_RESPONSE_MAX_BYTES`), 엔트리당 본문 1 MiB 상한
     (거대 diff 하나가 캐시를 밀어내지 않도록). TTL(`AXGIT_CACHE_RESPONSE_TTL`)은 검증자가
     못 보는 변경(config 수동 편집 등)의 staleness 상한 역할.
   - 제외 대상: repos 목록은 스캔 스냅샷이라 `ScanCache`(단일 값 TTL)를 그대로 쓰고,
     raw는 대용량 바이너리, archive는 스트리밍이라 캐시하지 않는다.
2. **클라이언트 캐시** — sha 포함 URL은 `immutable`, 그 외 `ETag`(검증자 기반) + `no-cache` + 304.

캐시 계층은 tower 미들웨어가 아니라 핸들러 공통 헬퍼(`handlers/mod.rs::cached_response`)로 둔다:
immutable 여부가 ref 해석 후에야 결정되고, params 정규화와 content-type이 엔드포인트마다 다르며,
에러 응답은 캐시하면 안 되기 때문.

주의: git2 `Repository`는 `Sync`가 아니다. 캐시에는 직렬화된 응답만 저장하고, Repository는 요청 스코프에서 open한다.

### Smart HTTP

- advertise: `git upload-pack --stateless-rpc --advertise-refs {repo}` 출력 앞에 pkt-line 서비스 헤더 부착.
- 데이터: 요청 body를 `git upload-pack --stateless-rpc {repo}` stdin으로, stdout을 응답으로 스트리밍.
  gzip 요청 body(`Content-Encoding: gzip`) 해제 처리 필요.
- receive-pack은 어떤 형태로든 노출하지 않는다 (403).

### 테스트 전략

- `api/tests/`에서 tempdir에 git CLI로 fixture bare repo 생성(커밋/태그/서브모듈 포함) 후
  axum 라우터에 대해 통합 테스트. 스냅샷/클론은 실제 `git clone http://…`으로 round-trip 검증.

## 프론트엔드 (web/)

- **Astro static** + React islands + shadcn/ui + Tailwind. 데이터는 전부 client-side fetch.
- 라우트 구성 (Astro 페이지는 껍데기, 데이터 로딩은 island):
  - `/` 저장소 목록 (section별 그룹핑, cgit index 대응)
  - `/{repo}/` summary · `/{repo}/log` · `/{repo}/tree/[...path]` · `/{repo}/blob/[...path]`
  - `/{repo}/commit/{sha}` · `/{repo}/refs` · `/{repo}/blame/[...path]`
  - ref 선택은 URL 쿼리 `?ref=` 통일
- 코드 하이라이팅: **Shiki client-side**, 언어 grammar lazy-load. 대용량 파일은 하이라이팅 생략 임계값 둠.
  (Astro 내장 Shiki/markdown은 빌드 타임 전용이라 런타임 fetch 데이터에는 쓸 수 없음.)
- README 렌더링: **react-markdown + remark-gfm + rehype-sanitize** (markdown만, rst/plain은 `<pre>` 표시).
- 아바타: 커밋 작성자 이메일 해시를 seed로 **DiceBear** 로컬 생성 (외부 요청 없음). 커밋 메시지 링크화는 정규식 linkify.
- vitest + Testing Library. API 클라이언트는 fetch mocking으로, 컴포넌트는 fixture JSON으로 테스트.
- 정적 빌드이므로 저장소별 페이지는 dynamic route 1벌 + client fetch로 해결 (빌드 시 저장소 목록 불필요).

## 빌드/배포

- 멀티스테이지 Dockerfile: ① node:alpine에서 `pnpm --filter web build` → ② rust:alpine에서
  `cargo build --release` → ③ alpine 런타임: git 바이너리 + api 바이너리 + web/dist.
  ①에서는 루트 `package.json`/`pnpm-workspace.yaml`/`pnpm-lock.yaml` + `web/package.json`을 먼저
  COPY해 `pnpm install --frozen-lockfile`을 레이어 캐시에 태운 뒤 나머지 소스를 COPY한다.
- 런타임 이미지에 필요한 패키지: `git`(exec용), `ca-certificates`. Python/pygments/groff 등 cgit 필터 의존성은 전부 불필요.
- 설정은 환경변수: `AXGIT_REPO_ROOT`, `AXGIT_STATIC_DIR`, `AXGIT_LISTEN`(기본 `0.0.0.0:8080`),
  `AXGIT_CLONE_URL_BASE`(clone URL 표시용), `AXGIT_CACHE_SCAN_TTL`(저장소 스캔 TTL, 기본 60s),
  `AXGIT_CACHE_RESPONSE_TTL`(응답 캐시 TTL, 기본 300s),
  `AXGIT_CACHE_RESPONSE_MAX_BYTES`(응답 캐시 용량, 기본 32 MiB).
- 로그는 stdout/stderr JSON(`tracing` + `tracing-subscriber`) — 스택의 fluentd logging driver가 수집.
