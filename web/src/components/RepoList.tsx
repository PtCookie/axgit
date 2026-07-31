import { useEffect, useState } from "react";

import { ApiError } from "@/lib/api/client";
import { listRepos } from "@/lib/api/repos";
import type { RepoInfo } from "@/lib/api/schemas";
import { formatAbsoluteTime, formatRelativeTime } from "@/lib/format/time";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

const UNSECTIONED_LABEL = "기타";

interface RepoGroup {
  section: string | null;
  repos: RepoInfo[];
}

function groupBySection(repos: RepoInfo[]): RepoGroup[] {
  const groups: RepoGroup[] = [];

  for (const repo of repos) {
    const existing = groups.find((group) => group.section === repo.section);
    if (existing) {
      existing.repos.push(repo);
    } else {
      groups.push({ section: repo.section, repos: [repo] });
    }
  }

  // section: null("기타")은 항상 맨 뒤. 나머지는 최초 등장 순서(= repos 정렬 순서)를 유지한다.
  groups.sort((a, b) => (a.section === null ? 1 : b.section === null ? -1 : 0));
  return groups;
}

type State = { status: "loading" } | { status: "error"; error: ApiError } | { status: "data"; repos: RepoInfo[] };

export default function RepoList() {
  const [state, setState] = useState<State>({ status: "loading" });

  useEffect(() => {
    let cancelled = false;

    listRepos()
      .then((response) => {
        if (!cancelled) {
          setState({ status: "data", repos: response.repos });
        }
      })
      .catch((error: unknown) => {
        if (cancelled) {
          return;
        }
        setState({
          status: "error",
          error: error instanceof ApiError ? error : new ApiError("internal", "unknown error", 0),
        });
      });

    return () => {
      cancelled = true;
    };
  }, []);

  if (state.status === "loading") {
    return (
      <div className="space-y-2" aria-busy="true">
        <Skeleton className="h-8 w-full" />
        <Skeleton className="h-8 w-full" />
        <Skeleton className="h-8 w-full" />
      </div>
    );
  }

  if (state.status === "error") {
    return (
      <p role="alert" className="text-destructive text-sm">
        저장소 목록을 불러오지 못했습니다: {state.error.message}
      </p>
    );
  }

  if (state.repos.length === 0) {
    return <p className="text-muted-foreground text-sm">등록된 저장소가 없습니다.</p>;
  }

  return (
    <div className="space-y-8">
      {groupBySection(state.repos).map(({ section, repos }) => (
        <section key={section ?? UNSECTIONED_LABEL}>
          <h2 className="text-muted-foreground mb-2 text-sm font-medium">{section ?? UNSECTIONED_LABEL}</h2>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>이름</TableHead>
                <TableHead>설명</TableHead>
                <TableHead>소유자</TableHead>
                <TableHead>최근 활동</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {repos.map((repo) => (
                <TableRow key={repo.name}>
                  <TableCell className="font-medium">{repo.name}</TableCell>
                  <TableCell className="text-muted-foreground">{repo.description ?? "—"}</TableCell>
                  <TableCell className="text-muted-foreground">{repo.owner ?? "—"}</TableCell>
                  <TableCell className="text-muted-foreground">
                    {repo.last_modified ? (
                      <span title={formatAbsoluteTime(repo.last_modified)}>
                        {formatRelativeTime(repo.last_modified)}
                      </span>
                    ) : (
                      "—"
                    )}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </section>
      ))}
    </div>
  );
}
