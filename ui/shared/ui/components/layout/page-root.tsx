import type { ReactNode } from "react";

import { cn } from "#utils/cn.ts";

export function PageRoot({
  scrollable = true,
  children,
}: {
  scrollable?: boolean;
  children: ReactNode;
}) {
  return (
    <div
      className={cn("h-full w-full", {
        "overflow-y-auto": scrollable,
      })}
    >
      {children}
    </div>
  );
}
