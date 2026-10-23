"use client";

import type { RefObject } from "react";
import { useLayoutEffect, useRef, useState } from "react";

/**
 * A custom React hook to determine if the content of a given HTML element is truncated.
 *
 * @template T - The type of the HTML element. Defaults to `HTMLElement`.
 * @param content - The content to be checked for truncation.
 * @returns A tuple containing a ref object for the HTML element and a boolean indicating whether the content is truncated.
 *
 * @example
 * ```tsx
 * const MyComponent = () => {
 *   const [ref, isTruncated] = useIsTruncated("Long content that might be truncated.");
 *
 *   return (
 *     <div ref={ref} style={{ width: '100px', overflow: 'hidden', whiteSpace: 'nowrap', textOverflow: 'ellipsis' }}>
 *       {isTruncated ? "Content is truncated" : "Content is not truncated"}
 *     </div>
 *   );
 * };
 * ```
 */
export function useIsTruncated<T extends HTMLElement = HTMLElement>(
  content: string,
): [RefObject<T | null>, boolean] {
  const ref = useRef<T | null>(null);
  const [isTruncated, setIsTruncated] = useState<boolean>(false);

  useLayoutEffect(() => {
    const element = ref.current;
    setIsTruncated(
      Boolean(element && element.scrollWidth > element.clientWidth),
    );
  }, [content]);

  return [ref, isTruncated];
}
