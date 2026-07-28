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

## 다음 구현: 커밋 로그

### Context

DECISIONS.md #9 순서(index → summary → refs → **log** → tree → …)의 다음 단계.
범위: `GET /api/v1/repos/{repo}/commits?ref=&path=&cursor=&limit=` (`docs/API.md` 참고).
커밋 상세(`/commits/{sha}`)와 diff는 범위가 크므로 별도 단계로 나눈다.

### 착수 시 검토할 것 (전 세션에서 확정하지 않음 — 착수 시점에 설계할 것)

- `ref` 파라미터 해석(브랜치/태그/sha → commit)을 공통 헬퍼로: 이후 tree/blob/raw가 재사용.
  실패 시 `ApiError::RefNotFound`.
- cursor 페이지네이션: cursor = 커밋 sha, `revwalk`를 cursor부터 시작해 `limit + 1`개 걷고
  마지막을 `next_cursor`로. `limit` 기본 50, 최대 100 (초과 시 400 또는 clamp — API.md에 명시).
- `path` 필터: libgit2 revwalk에는 pathspec 필터가 없어 diff 검사로 직접 구현하거나
  `git log` exec로 처리. 성능 특성을 보고 결정 (Gitea식 하이브리드 허용).
- `author.email_hash`(sha256) — 이메일 원문 비노출 (API.md).

## 이후 항목 (DECISIONS.md #9 순서, 착수 전 재검토 필요)

- 커밋 상세 + diff (`GET /commits/{sha}`, `/commits/{sha}/diff`).
- tree/blob/raw + README — 스케폴딩에서 유예한 `{ref}/{path...}` 라우팅 모호성(브랜치명의 `/`)을
  여기서 실제로 풀어야 함 (`api/src/routes.rs`의 wildcard 라우트 주석 참고).
- blame (v1 포함, 구현 순서는 마지막 — API.md 명시).
- archive(`git archive` exec), Atom feed.
- Smart HTTP upload-pack (`api/src/smart_http.rs`에 설계 요약만 있음, 현재 501/403 stub만 존재).
- 이 시점부터 moka 기반 (repo, endpoint, params) 응답 캐시 도입 검토
  (`api/src/cache.rs` 상단 주석 및 `docs/ARCHITECTURE.md#caching` 참고).
  **API.md의 ETag/Cache-Control 규약(L29-30)도 아직 미구현** — 검증자(HEAD sha)를 쓰는
  응답 캐시와 함께 이 시점에 일괄 구현한다 (summary/refs 구현 시 의도적으로 유예).
