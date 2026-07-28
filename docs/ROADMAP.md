# Axgit 구현 로드맵

`api/`의 엔드포인트를 어떤 순서로, 어떤 설계로 구현할지 세션 간에 이어가기 위한 문서.
완료 시 다음 절을 갱신하고, "다음 구현" 절을 그 다음 대상으로 교체할 것.

## 완료됨

- `GET /api/v1/repos` — 저장소 스캔(`api/src/repo/scan.rs`, `meta.rs`), TTL 캐시(`cache.rs`),
  `[axgit]`/`[cgit]` 메타데이터 읽기. 나머지 엔드포인트는 501 stub, `git-receive-pack`은 403.

## 다음 구현: 저장소 요약 + refs

### Context

목록(index) 다음의 자연스러운 단계. `docs/DECISIONS.md` #9의 v1 순서(index → summary → refs →
log → tree → blob/raw → commit/diff → …)와도 일치한다. 이 둘을 먼저 하는 이유는 이후 모든
엔드포인트(`commits`, `tree`, `blob`, `raw`, `blame`)가 공통으로 필요로 하는 두 가지 기반을
여기서 확정할 수 있기 때문:

1. **`{repo}` 경로 파라미터로 저장소를 여는 공통 로직** — 없으면 `repo_not_found`,
   사용자 입력이므로 path traversal 방지 필요.
2. **ref(브랜치/태그/sha) 나열과 해석** — `refs` 엔드포인트가 그 자체로 이 기반이다.

범위: `GET /api/v1/repos/{repo}`, `GET /api/v1/repos/{repo}/refs`. (`docs/API.md` L59-70)

### 설계 결정

1. **저장소 오픈 공통 헬퍼** — `api/src/repo/open.rs`:
   ```rust
   pub fn open_named(root: &Path, name: &str) -> Result<Repository, ApiError>
   ```
   - `name`에 `/`, `\`, 빈 문자열, `.`로 시작하는 값이 있으면 `ApiError::InvalidParam`으로 거부
     (path traversal 방지 — `{repo}`는 URL에서 온 사용자 입력).
   - `root.join(format!("{name}.git"))`을 bare open, 실패 시 `ApiError::RepoNotFound(name)`.
   - `handlers/repos.rs`의 두 핸들러가 공통으로 사용. `scan.rs`는 디렉터리 목록에서 이름을
     얻으므로 이 검증이 필요 없다(기존 코드 변경 없음).

2. **refs 조회를 별도 모듈로 분리, summary가 재사용** — `api/src/repo/refs.rs`:
   ```rust
   #[derive(Serialize)] pub struct BranchRef { name, target, committed_at }
   #[derive(Serialize)] pub struct TagRef { name, target, annotation: Option<String>, tagged_at: Option<String> }
   #[derive(Serialize)] pub struct RefsInfo { branches: Vec<BranchRef>, tags: Vec<TagRef> }
   pub fn list_refs(repo: &Repository) -> anyhow::Result<RefsInfo>
   ```
   - 브랜치: `repo.branches(Some(BranchType::Local))`, `committed_at`은 대상 커밋의 authordate
     (`meta.rs`의 시각 포맷 로직 재사용/공유 — `jiff` 포맷 함수를 `meta.rs`에서 `pub(crate)`로 빼서
     공유하는 편이 중복보다 낫다).
   - 태그: `repo.tag_names(None)` 순회 후 `repo.find_reference("refs/tags/{name}")` →
     peel. **경량 태그(lightweight tag)는 `git2::Tag` 오브젝트가 없어 `annotation`/`tagged_at`이
     없다** — `Option`로 두고 `docs/API.md`에 nullable 명시 필요 (현재 문서에 미기재된 부분).
   - `repo/mod.rs`의 summary 핸들러는 `list_refs`의 `branches.len()`/`tags.len()`으로 카운트만
     사용, 전체 목록을 다시 순회하지 않는다.

3. **저장소 요약 응답** — `handlers/repos.rs::get_repo`:
   - `repo/mod.rs::RepoInfo`의 필드(section/owner/desc/default_branch/last_modified)는
     `meta::read_repo_info`를 그대로 재사용해 얻고, 여기에 `head`(sha, `Option<String>`,
     unborn HEAD면 null), `branch_count`, `tag_count`, `clone_url`(`Option<String>`)을 얹은
     `RepoSummary` 구조체를 새로 정의. `RepoInfo`를 억지로 확장하지 말고 별도 타입으로 둘 것
     (목록 응답과 요약 응답은 API 계약이 다르므로).
   - `clone_url`: `state.config.clone_url_base`가 있으면 `format!("{base}/{repo}.git")`,
     없으면 `null`. **이 필드는 `docs/API.md`에 형식이 전혀 기재돼 있지 않으므로 구현 시 함께
     추가할 것** (CLAUDE.md 규칙: API 변경은 같은 커밋에서 API.md 갱신).

4. **빈 저장소(unborn HEAD)는 404가 아니다** — 목록 엔드포인트와 동일하게 `head: null`,
   `default_branch: null`, `branches: []`, `tags: []`로 200 응답. `repo_not_found`는 디렉터리
   자체가 없거나 열 수 없을 때만.

### 파일 변경 목록

| 파일 | 변경 |
|---|---|
| `api/src/repo/open.rs` (신규) | `open_named` — 이름 검증 + bare open + 404 매핑 |
| `api/src/repo/refs.rs` (신규) | `BranchRef`/`TagRef`/`RefsInfo`/`list_refs` |
| `api/src/repo/meta.rs` | 시각 포맷 함수(`format_rfc3339` 등)를 `pub(crate)`로 공개해 `refs.rs`와 공유 |
| `api/src/repo/mod.rs` | `pub mod open; pub mod refs;` 추가, `RepoSummary` 구조체 정의 |
| `api/src/handlers/repos.rs` | `get_repo`, `get_refs` 핸들러 추가 |
| `api/src/routes.rs` | `/repos/{repo}`, `/repos/{repo}/refs`를 501 stub에서 실제 핸들러로 교체 |
| `api/tests/common/mod.rs` | `add_branch`(두 번째 브랜치), `add_annotated_tag`, `add_lightweight_tag` 헬퍼 추가 |
| `api/tests/repo_test.rs` (신규) | 아래 "검증" 절의 케이스들 |
| `docs/API.md` | `clone_url` 필드 형식, 태그 `annotation`/`tagged_at` nullable, 빈 저장소 응답 형태 명시 |

### 구현 순서

1. `meta.rs`에서 시각 포맷 함수 공개 → `repo/open.rs` (이름 검증 단위 테스트 먼저 작성해도 됨)
2. `repo/refs.rs` (`list_refs`) — 브랜치만 먼저, 태그(경량/annotated 구분) 추가
3. `repo/mod.rs::RepoSummary` + `handlers/repos.rs::get_repo`/`get_refs`
4. `routes.rs` 라우트 교체
5. `tests/common/mod.rs` 헬퍼 확장 → `tests/repo_test.rs`
6. clippy/fmt + `docs/API.md` 갱신 (같은 커밋)

### 검증

```sh
cargo test --manifest-path api/Cargo.toml
cargo clippy --manifest-path api/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path api/Cargo.toml -- --check

./scripts/make-fixtures.sh
cargo run --manifest-path api/Cargo.toml -- --repo-root ./fixtures/repos --clone-url-base http://git.example.com
curl -s localhost:8080/api/v1/repos/git-compose | jq .        # head/branch_count/tag_count/clone_url
curl -s localhost:8080/api/v1/repos/git-compose/refs | jq .    # branches[]/tags[] (annotated tag 포함)
curl -si localhost:8080/api/v1/repos/does-not-exist | head -2 # 404 repo_not_found
curl -si "localhost:8080/api/v1/repos/..%2f..%2fetc" | head -2 # 400 invalid_param (path traversal 차단)
```

테스트 케이스(`api/tests/repo_test.rs`):
- `get_repo`: 존재하는 저장소 → head sha, branch_count, tag_count, clone_url 확인
- `get_repo`: 존재하지 않는 저장소 → 404 `repo_not_found`
- `get_repo`: `{repo}`에 `/` 또는 `..` 포함 → 400 `invalid_param`
- `get_repo`: 빈 저장소 → 200, head/default_branch null, branch_count/tag_count 0
- `get_refs`: 브랜치 2개 + annotated 태그 1개 → 배열과 target sha 확인
- `get_refs`: 경량 태그 → `annotation`/`tagged_at`이 null
- `get_refs`: 빈 저장소 → `branches: []`, `tags: []`

## 이후 항목 (DECISIONS.md #9 순서, 착수 전 재검토 필요)

- 커밋 로그 (`GET /commits`, cursor 페이지네이션) + 커밋 상세 + diff — 범위가 크므로 로그/상세/diff를
  다시 나눌지 판단 필요.
- tree/blob/raw + README — 스케폴딩에서 유예한 `{ref}/{path...}` 라우팅 모호성(브랜치명의 `/`)을
  여기서 실제로 풀어야 함 (`api/src/routes.rs`의 wildcard 라우트 주석 참고).
- blame (v1 포함, 구현 순서는 마지막 — API.md 명시).
- archive(`git archive` exec), Atom feed.
- Smart HTTP upload-pack (`api/src/smart_http.rs`에 설계 요약만 있음, 현재 501/403 stub만 존재).
- 이 시점부터 moka 기반 (repo, endpoint, params) 응답 캐시 도입 검토
  (`api/src/cache.rs` 상단 주석 및 `docs/ARCHITECTURE.md#caching` 참고).
