"use client";

import { ChevronRightIcon } from "lucide-react";
import type { ReactNode } from "react";

import { cn } from "#utils/cn.ts";

import type { CollapsibleSize } from "./collapsible";
import { useCollapsibleContext } from "./collapsible-context";

const iconSizeStyles: Record<CollapsibleSize, string> = {
  xs: "size-3",
  sm: "size-3.5",
  md: "size-4",
  lg: "size-5",
};

interface CollapsibleTriggerProps {
  hideIcon?: boolean;
  iconClassName?: string;
  className?: string;
  children: ReactNode;
}

const CollapsibleTrigger = ({
  hideIcon = false,
  iconClassName,
  className,
  children,
}: CollapsibleTriggerProps) => {
  const { isOpen, toggle, contentId, triggerId, size, fullWidth } =
    useCollapsibleContext();

  return (
    <button
      type="button"
      id={triggerId}
      aria-expanded={isOpen}
      aria-controls={contentId}
      onClick={toggle}
      className={cn(fullWidth && "flex w-full justify-between", className)}
    >
      {children}

      {hideIcon ? null : (
        <ChevronRightIcon
          className={cn(
            "mt-0.5 shrink-0 transition-transform duration-200",
            iconSizeStyles[size],
            isOpen && "rotate-90",
            iconClassName,
          )}
        />
      )}
    </button>
  );
};

CollapsibleTrigger.displayName = "CollapsibleTrigger";

export type { CollapsibleTriggerProps };
export { CollapsibleTrigger };
