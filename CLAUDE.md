# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Overview

**Axgit**는 self-hosted Git 서버([git-compose](https://git.ptcookie.net/git-compose.git) 스택)의
Cgit을 대체하는 웹 프론트엔드입니다. 모노레포로 두 컴포넌트를 관리합니다:

- **api/** — Rust 백엔드 (axum + git2/libgit2). bare 저장소를 읽어 JSON API 제공,
  Smart HTTP clone(`git-upload-pack`) 서빙, Astro 정적 빌드 결과물 서빙까지 담당.
- **web/** — Astro + React + shadcn/ui 프론트엔드. 정적(static) 빌드 후 API를 client-side fetch.

**배포는 단일 컨테이너**: 멀티스테이지 Dockerfile에서 web 빌드 → api 빌드 → 런타임 이미지 하나.
기존 git-compose 스택의 `git-web` 서비스를 이 이미지로 교체하며, nginx/fcgiwrap/CGI는 사용하지 않는다.

## 핵심 제약 (Invariants)

- **웹은 철저히 읽기 전용.** 쓰기(push, 저장소 생성)는 git-server 컨테이너의 SSH로만 이루어진다.
  API에 mutation 엔드포인트를 추가하지 말 것. 인증/권한 로직도 없다.
- **저장소는 `/srv/git`의 bare repo**이며 컨테이너에 read-only 마운트된다 (`AXGIT_REPO_ROOT`로 설정).
- **저장소 메타데이터는 각 repo의 `config` 파일 `[cgit]` 섹션** (`section`, `name`, `owner`, `desc`)에서 읽는다.
  git-server의 `git-init` 스크립트가 이 형식으로 쓰기 때문에 **호환성을 깨지 말 것**. `[axgit]` 섹션이 있으면 우선한다.
- **최근 활동 시각은 agefile** (`info/web/last-modified`)에서 읽는다. git-server의 post-receive 훅이 갱신한다.
  agefile이 없으면 HEAD 커밋의 authordate로 fallback.
- Smart HTTP는 **upload-pack(fetch/clone)만** 지원. `git-receive-pack` 요청은 403으로 거부한다.

## Commands

```sh
# 최초 설정
pnpm install                # web 의존성 (workspace root에서)
lefthook install            # git hooks 등록

# Frontend (web/)
pnpm --filter web dev       # Astro dev 서버 (API는 AXGIT_API_URL proxy)
pnpm --filter web build     # 정적 빌드 → web/dist/
pnpm --filter web test      # vitest
pnpm --filter web lint      # eslint + prettier check

# Backend (api/)
cargo build --manifest-path api/Cargo.toml
cargo test --manifest-path api/Cargo.toml
cargo clippy --manifest-path api/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path api/Cargo.toml

# 로컬 통합 실행 (api가 web/dist를 서빙)
cargo run --manifest-path api/Cargo.toml -- --repo-root ./fixtures/repos --static-dir ./web/dist

# 컨테이너 빌드
docker build --tag axgit:latest .
```

## Architecture

상세 설계는 `docs/ARCHITECTURE.md`, API 계약은 `docs/API.md`, 결정 이력은 `docs/DECISIONS.md` 참고.
**API를 변경할 때는 반드시 `docs/API.md`를 같은 커밋에서 갱신할 것** — 이 문서가 web/api 간 유일한 계약이다.

### 백엔드 요점

- git2의 `Repository`는 `Sync`가 아니므로 **요청마다 open**한다 (open 비용은 낮음). 전역 캐시에 Repository를 넣지 말 것.
- 무거운 연산은 git 바이너리 exec로 처리한다: 아카이브는 `git archive`,
  Smart HTTP는 `git http-backend`(CGI 방식 spawn) 또는 `git upload-pack --stateless-rpc`.
  libgit2로 전부 구현하려 하지 말 것 (Gitea도 같은 하이브리드 패턴).
- **캐싱은 cgit 스타일 + 개선**: 응답 캐시를 (repo, endpoint, params) 키로 TTL 저장하되,
  단순 TTL이 아니라 repo의 HEAD/agefile mtime을 검증자로 사용해 push 후 즉시 무효화한다.
  클라이언트 캐시는 ETag(커밋 sha 기반)로 처리. 상세는 `docs/ARCHITECTURE.md#caching`.

### 프론트엔드 요점

- Astro는 **static 모드** 고정. SSR adapter를 추가하지 말 것 (배포 형태가 바뀌는 결정이므로 논의 필요).
- 동적 데이터는 React island에서 API fetch. 라우팅은 Astro 페이지 + URL 쿼리/경로 파라미터.
- 코드 하이라이팅은 Shiki를 client-side에서 lazy-load (언어별 동적 import).
  README는 react-markdown + rehype-sanitize, 아바타는 DiceBear 로컬 생성 (외부 요청 금지, DECISIONS.md #11).
- UI 컴포넌트는 shadcn/ui 사용, `web/src/components/ui/`에 생성. 생성된 파일은 수정 가능(vendored 방식).

## Conventions

- **커밋 메시지**: Conventional Commits 영어 (`feat:`, `fix:`, `build:`, `docs:`, `refactor:`, `test:`).
  scope는 필요 시 `feat(api):`, `fix(web):` 형태.
- **Rust**: edition 2024, `cargo fmt` 기본 설정, clippy 경고 0 유지 (`-D warnings`).
  에러는 `thiserror`(라이브러리 코드) + `anyhow`(bin 진입부).
- **TypeScript**: strict 모드. API 응답 타입은 `web/src/lib/api/types.ts`에 수동 정의하고
  `docs/API.md`와 동기화한다 (코드 생성 도입 전까지).
- **테스트**: api는 tempdir에 git CLI로 fixture repo를 만들어 통합 테스트 (`api/tests/`),
  web은 vitest + Testing Library. 커밋 전 훅은 lefthook이 담당 (`lefthook.yml`).
- 사용자와의 대화는 한국어, 코드/커밋/문서 식별자는 영어.

## Repository layout

```
axgit/
  api/                # Rust crate (axum + git2)
    src/
      main.rs         # 진입점, 라우터 구성
      repo/           # 저장소 스캔, 메타데이터, git2 읽기
      handlers/       # HTTP 핸들러 (API.md와 1:1)
      cache.rs        # 응답 캐시
      smart_http.rs   # git-upload-pack 프록시
    tests/            # fixture repo 기반 통합 테스트
  web/                # Astro + React + shadcn/ui
    src/
      pages/          # Astro 라우트
      components/     # React islands, ui/ (shadcn)
      lib/api/        # fetch 클라이언트 + 타입
  docs/               # ARCHITECTURE.md, API.md, DECISIONS.md
  lefthook.yml
  Dockerfile          # web build → api build → runtime (단일 이미지)
```
