"use client";

import { useIsClient, useMediaQuery } from "usehooks-ts";

export type Breakpoint = "xs" | "sm" | "md" | "lg" | "xl" | "2xl";

interface BreakpointInformation {
  isSm: boolean;
  isMd: boolean;
  isLg: boolean;
  isXl: boolean;
  is2xl: boolean;
  breakpoint: Breakpoint;
}

export function useBreakpoint(): BreakpointInformation | null {
  const isClient = useIsClient();

  // Get the :root element's computed styles
  const rootStyles = isClient
    ? getComputedStyle(document.documentElement)
    : null;
  const getBreakpointMediaQuery = (breakpoint: string) =>
    rootStyles
      ? `(min-width: ${rootStyles.getPropertyValue(`--breakpoint-${breakpoint}`)})`
      : "";

  const isSm = useMediaQuery(getBreakpointMediaQuery("sm"));
  const isMd = useMediaQuery(getBreakpointMediaQuery("md"));
  const isLg = useMediaQuery(getBreakpointMediaQuery("lg"));
  const isXl = useMediaQuery(getBreakpointMediaQuery("xl"));
  const is2xl = useMediaQuery(getBreakpointMediaQuery("2xl"));

  // Return null if the component is rendered on the server and let the caller
  // handle the null case.
  if (!isClient) {
    return null;
  }

  const breakpoint: Breakpoint = is2xl
    ? "2xl"
    : isXl
      ? "xl"
      : isLg
        ? "lg"
        : isMd
          ? "md"
          : isSm
            ? "sm"
            : "xs";

  return { isSm, isMd, isLg, isXl, is2xl, breakpoint };
}
