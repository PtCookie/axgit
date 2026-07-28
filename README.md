# Axgit

Self-hosted Git 서버(git-compose 스택)를 위한 웹 프론트엔드. Cgit + Nginx + fcgiwrap 구성을 대체한다.

- **api/** — Rust (axum + libgit2). 읽기 전용 JSON API, Smart HTTP clone, 정적 파일 서빙.
- **web/** — Astro + React + shadcn/ui. 정적 빌드.

단일 컨테이너로 배포하며, push는 기존 git-server(SSH)가 담당한다.

문서: [아키텍처](docs/ARCHITECTURE.md) · [API 명세](docs/API.md) · [결정 이력](docs/DECISIONS.md)

## 개발

```sh
pnpm install && lefthook install
pnpm --filter web dev                        # frontend dev 서버
cargo run --manifest-path api/Cargo.toml     # backend
```
