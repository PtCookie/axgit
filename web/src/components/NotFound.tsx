/** Client-side route that matched nothing (as opposed to an API 404). */
export default function NotFound() {
  return (
    <div className="space-y-2">
      <h1 className="font-heading text-2xl font-medium">페이지를 찾을 수 없습니다</h1>
      <p className="text-muted-foreground text-sm">
        <a href="/" className="text-primary underline-offset-4 hover:underline">
          저장소 목록으로 돌아가기
        </a>
      </p>
    </div>
  );
}
