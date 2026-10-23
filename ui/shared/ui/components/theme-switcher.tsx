"use client";

import { Moon, Sun } from "lucide-react";
import { useTheme } from "next-themes";
import { useIsClient } from "usehooks-ts";

import { Button } from "./ds/button";

export function ThemeSwitcher() {
  const { resolvedTheme, setTheme } = useTheme();
  const isClient = useIsClient();

  if (!isClient || !resolvedTheme) {
    return (
      <Button
        aria-label="Toggle theme"
        variant="ghost"
        mode="icon-only"
        icon={<span className="h-4 w-4" />}
        className="h-7 w-7 rounded-full"
        isDisabled
      />
    );
  }

  const nextTheme = resolvedTheme === "light" ? "dark" : "light";

  return (
    <Button
      aria-label="Toggle theme"
      variant="ghost"
      mode="icon-only"
      icon={
        resolvedTheme === "light" ? (
          <Moon className="h-4 w-4" />
        ) : (
          <Sun className="h-4 w-4" />
        )
      }
      className="h-7 w-7 rounded-full"
      onClick={() => setTheme(nextTheme)}
    />
  );
}
