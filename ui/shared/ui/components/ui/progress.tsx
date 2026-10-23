"use client";

import * as ProgressPrimitive from "@radix-ui/react-progress";
import * as React from "react";

import { cn } from "#utils/cn.ts";

const Progress = ({
  ref,
  className,
  value,
  ...props
}: React.ComponentProps<typeof ProgressPrimitive.Root>) => (
  <ProgressPrimitive.Root
    ref={ref}
    className={cn(
      "bg-secondary relative h-4 w-full overflow-hidden rounded-full",
      className,
    )}
    {...props}
  >
    <ProgressPrimitive.Indicator
      className="bg-state-brand h-full w-full flex-1 transition-all"
      style={{ transform: `translateX(-${100 - (value || 0)}%)` }}
    />
  </ProgressPrimitive.Root>
);
Progress.displayName = ProgressPrimitive.Root.displayName;

export { Progress };

export const LeChatProgressBar = ({
  value,
  total,
  className,
}: {
  value: number;
  total: number;
  className?: string;
}) => {
  return (
    <ProgressPrimitive.Root
      value={value}
      max={Math.max(total, value)}
      className={cn(
        "relative h-[4px] w-full overflow-hidden rounded-full bg-(--secondary-300)",
        className,
      )}
    >
      <ProgressPrimitive.Indicator
        className="bg-brand-500 h-full w-full transition"
        style={{
          transform: `translateX(-${((total - value) * 100) / total}%)`,
        }}
      />
    </ProgressPrimitive.Root>
  );
};
