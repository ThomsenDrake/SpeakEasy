"use client";

import { PanelLeft } from "lucide-react";

import { cn } from "#utils/cn.ts";

import * as NavigationBar from "./index";

export function ToggleMenuButton({
  tooltipText,
  className,
  onClick,
  disabled = false,
  variant = "default",
}: {
  tooltipText?: string;
  className?: string;
  onClick?: () => void;
  disabled?: boolean;
  variant?: "default" | "inverted";
}) {
  return (
    <NavigationBar.Button
      className={cn(
        "h-10 w-10 p-2.5",
        variant === "inverted" &&
          "hover:not-disabled:bg-(--zinc-700) active:not-disabled:bg-(--zinc-700)",
        className,
      )}
      tooltipText={tooltipText}
      onClick={onClick}
      disabled={disabled}
    >
      <PanelLeft
        className={cn(
          "size-[18px]",
          variant === "inverted" && "text-icon-inverted-default opacity-50",
        )}
        strokeWidth={1.75}
      />
    </NavigationBar.Button>
  );
}
