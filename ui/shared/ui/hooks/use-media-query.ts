"use client";

import { useEffect, useState } from "react";

/**
 * Custom hook that returns whether a media query matches.
 * @param query The media query to check
 * @returns A boolean indicating whether the media query matches
 */
export function useMediaQuery(query: string): boolean {
  const [matches, setMatches] = useState(false);

  useEffect(() => {
    if (typeof window === "undefined") return;

    const mediaQuery = window.matchMedia(query);

    const updateMatches = () => {
      setMatches(mediaQuery.matches);
    };

    // Set initial value
    updateMatches();

    // Listen for changes
    mediaQuery.addEventListener("change", updateMatches);

    // Clean up
    return () => {
      mediaQuery.removeEventListener("change", updateMatches);
    };
  }, [query]);

  return matches;
}
