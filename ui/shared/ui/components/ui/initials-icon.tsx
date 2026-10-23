import { User } from "lucide-react";
import * as React from "react";

import { cn } from "#utils/cn.ts";

export interface InitialsIconProps extends React.HTMLAttributes<HTMLDivElement> {
  name?: string | null;
  size?: "sm" | "md" | "lg";
}

const sizeClasses = {
  sm: "size-6 text-[10px]",
  md: "size-8 text-xs",
  lg: "size-10 text-sm",
} as const;

export function InitialsIcon({
  name,
  size = "md",
  className,
  ...props
}: InitialsIconProps) {
  const initials = React.useMemo(() => {
    return name
      ?.split(" ")
      .map((n) => n[0])
      .slice(0, 2)
      .join("")
      .toUpperCase();
  }, [name]);

  return (
    <div
      className={cn(
        "text-muted dark:text-muted flex aspect-square items-center justify-center rounded-lg bg-(--secondary-200) font-semibold",
        sizeClasses[size],
        className,
      )}
      {...props}
    >
      {initials || (
        <User
          className={cn(
            size === "sm" ? "size-3" : size === "md" ? "size-4" : "size-5",
          )}
        />
      )}
    </div>
  );
}
