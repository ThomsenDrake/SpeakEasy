import { cn } from "#utils/cn.ts";

function Skeleton({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn(
        "bg-muted dark:bg-state-secondary animate-pulse rounded-md",
        className,
      )}
      {...props}
    />
  );
}

export { Skeleton };
