import { useRef } from "react";
import { isDeepEqual } from "remeda";

/**
 * A React hook that memoizes a value using deep equality comparison.
 *
 * Unlike `useMemo`, which only uses reference equality, this hook stores
 * the previous value and performs a deep comparison with the current one.
 * If the values are deeply equal, it returns the previous reference.
 * Otherwise, it updates and returns the new value.
 *
 * @template T - The type of the value being memoized.
 * @param value - The value to be memoized.
 * @returns The previous deeply equal value or the updated one if changed.
 *
 * @example
 * const memoizedData = useDeepEqualMemo(data);
 * useEffect(() => {
 *   // Only runs if `data` has changed deeply
 * }, [memoizedData]);
 */
export function useDeepEqualMemo<T>(value: T): T {
  const ref = useRef<T>(value);

  if (!isDeepEqual(ref.current, value)) {
    ref.current = value;
  }

  return ref.current;
}
