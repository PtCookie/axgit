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

## #7 도구 체인

- 패키지 매니저: **pnpm** (workspace).
- git hooks: **lefthook** — 단일 바이너리로 Rust/JS 훅을 한 설정에서 관리 (husky+lint-staged 대체).
- 테스트: web은 **vitest**, api는 cargo test + fixture repo 통합 테스트.
- 커밋: Conventional Commits (영어).

## #8 API 스타일

- REST JSON, base `/api/v1`. 계약 문서는 `docs/API.md` 단일 소스 (OpenAPI 도입은 추후 검토).
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
