import { useEffect, useState } from "react";

import { ApiError } from "@/lib/api/client";
import { getRefs } from "@/lib/api/repos";
import type { RefsInfo } from "@/lib/api/schemas";
import { formatAbsoluteTime, formatRelativeTime } from "@/lib/format/time";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

type State = { status: "loading" } | { status: "error"; error: ApiError } | { status: "data"; refs: RefsInfo };

interface RefsViewProps {
  repo: string;
}

export default function RefsView({ repo }: RefsViewProps) {
  const [state, setState] = useState<State>({ status: "loading" });

  useEffect(() => {
    let cancelled = false;

    getRefs(repo)
      .then((refs) => {
        if (!cancelled) {
          setState({ status: "data", refs });
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
  }, [repo]);

  if (state.status === "loading") {
    return (
      <div className="space-y-2" aria-busy="true">
        <Skeleton className="h-8 w-full" />
        <Skeleton className="h-8 w-full" />
      </div>
    );
  }

  if (state.status === "error") {
    const message = state.error.status === 404 ? "저장소를 찾을 수 없습니다." : state.error.message;
    return (
      <p role="alert" className="text-destructive text-sm">
        브랜치/태그 정보를 불러오지 못했습니다: {message}
      </p>
    );
  }

  const { refs } = state;

  return (
    <div className="space-y-8">
      <section>
        <h2 className="text-muted-foreground mb-2 text-sm font-medium">브랜치</h2>
        {refs.branches.length === 0 ? (
          <p className="text-muted-foreground text-sm">브랜치가 없습니다.</p>
        ) : (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>이름</TableHead>
                <TableHead>커밋</TableHead>
                <TableHead>커밋 시각</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {refs.branches.map((branch) => (
                <TableRow key={branch.name}>
                  <TableCell className="font-medium">{branch.name}</TableCell>
                  <TableCell className="text-muted-foreground font-mono">{branch.target.slice(0, 12)}</TableCell>
                  <TableCell className="text-muted-foreground">
                    {branch.committed_at ? (
                      <span title={formatAbsoluteTime(branch.committed_at)}>
                        {formatRelativeTime(branch.committed_at)}
                      </span>
                    ) : (
                      "—"
                    )}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        )}
      </section>

      <section>
        <h2 className="text-muted-foreground mb-2 text-sm font-medium">태그</h2>
        {refs.tags.length === 0 ? (
          <p className="text-muted-foreground text-sm">태그가 없습니다.</p>
        ) : (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>이름</TableHead>
                <TableHead>커밋</TableHead>
                <TableHead>메시지</TableHead>
                <TableHead>태그 시각</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {refs.tags.map((tag) => (
                <TableRow key={tag.name}>
                  <TableCell className="font-medium">{tag.name}</TableCell>
                  <TableCell className="text-muted-foreground font-mono">{tag.target.slice(0, 12)}</TableCell>
                  <TableCell className="text-muted-foreground">{tag.annotation ?? "—"}</TableCell>
                  <TableCell className="text-muted-foreground">
                    {tag.tagged_at ? (
                      <span title={formatAbsoluteTime(tag.tagged_at)}>{formatRelativeTime(tag.tagged_at)}</span>
                    ) : (
                      "—"
                    )}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        )}
      </section>
    </div>
  );
}
