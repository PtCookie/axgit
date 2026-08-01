/** Client-side route that matched nothing (as opposed to an API 404). */
export default function NotFound() {
  return (
    <div className="space-y-2">
      <h1 className="font-heading text-2xl font-medium">Page not found</h1>
      <p className="text-muted-foreground text-sm">
        <a href="/" className="text-primary underline-offset-4 hover:underline">
          Back to repository list
        </a>
      </p>
    </div>
  );
}
