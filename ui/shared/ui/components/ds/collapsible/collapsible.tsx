"use client";

import type { ReactNode } from "react";
import { useId, useState } from "react";

import { cn } from "#utils/cn.ts";

import { CollapsibleContext } from "./collapsible-context";

type CollapsibleVariant = "default" | "soft";
type CollapsibleSize = "xs" | "sm" | "md" | "lg";

interface CollapsibleProps {
  defaultOpen?: boolean;
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  variant?: CollapsibleVariant;
  size?: CollapsibleSize;
  fullWidth?: boolean;
  className?: string;
  children: ReactNode;
}

const collapsibleVariantStyles: Record<CollapsibleVariant, string> = {
  default: "",
  soft: "text-muted",
};

const collapsibleSizeStyles: Record<CollapsibleSize, string> = {
  xs: "text-xs",
  sm: "text-sm",
  md: "text-base",
  lg: "text-lg",
};

const Collapsible = ({
  defaultOpen = false,
  open,
  onOpenChange,
  variant = "default",
  size = "sm",
  fullWidth,
  className,
  children,
}: CollapsibleProps) => {
  const [internalOpen, setInternalOpen] = useState(defaultOpen);
  const isControlled = open !== undefined;
  const isOpen = isControlled ? open : internalOpen;
  const id = useId();
  const contentId = `collapsible-content-${id}`;
  const triggerId = `collapsible-trigger-${id}`;

  const toggle = () => {
    const next = !isOpen;
    if (!isControlled) {
      setInternalOpen(next);
    }
    onOpenChange?.(next);
  };

  return (
    <CollapsibleContext.Provider
      value={{ isOpen, toggle, contentId, triggerId, size, fullWidth }}
    >
      <div
        data-state={isOpen ? "open" : "closed"}
        className={cn(
          !fullWidth && "w-fit",
          collapsibleVariantStyles[variant],
          collapsibleSizeStyles[size],
          className,
        )}
      >
        {children}
      </div>
    </CollapsibleContext.Provider>
  );
};

Collapsible.displayName = "Collapsible";

export type { CollapsibleProps, CollapsibleSize, CollapsibleVariant };
export { Collapsible };
