# web

Axgit 프론트엔드. Astro(static) + React islands + shadcn/ui + Tailwind CSS.

명령은 리포지토리 루트의 pnpm workspace에서 `--filter web`으로 실행한다 (자세한 설명은
루트 [README.md](../README.md), [CLAUDE.md](../CLAUDE.md) 참고):

```sh
pnpm --filter web dev          # Astro dev 서버 (localhost:4321, /api는 AXGIT_API_URL로 프록시)
pnpm --filter web build        # 정적 빌드 → web/dist/
pnpm --filter web test         # vitest (browser mode)
pnpm --filter web test:e2e     # Playwright e2e
pnpm --filter web check        # eslint + prettier check
pnpm --filter web gen:types    # docs/openapi.json → src/lib/api/types.ts 재생성
```

## 구조

```text
web/
  src/
    pages/          # Astro 라우트
    layouts/        # 공용 레이아웃
    components/     # React islands, ui/ (shadcn, vendored)
    lib/
      api/          # fetch 클라이언트 + types.ts (openapi-typescript 생성, 직접 수정 금지)
      format/       # 날짜 등 표시 포맷 유틸
  tests/            # vitest (unit + browser mode 컴포넌트 테스트)
  e2e/              # Playwright
```

`src/lib/api/types.ts`는 `docs/openapi.json`에서 생성되는 파일이라 직접 편집하지 않는다
(eslint 대상에서도 제외). API가 바뀌면 `pnpm --filter web gen:types`로 재생성한다.
