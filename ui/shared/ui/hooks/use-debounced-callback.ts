import debounce from "lodash.debounce";
import React from "react";

import { useStableCallback } from "./use-stable-callback.ts";

/**
 * Reimplementation of the useDebonceCallback from usehooks-ts
 * see: https://github.com/juliencrn/usehooks-ts/blob/master/packages/usehooks-ts/src/useDebounceCallback/useDebounceCallback.ts
 *
 * Interface is kept similar to the original
 */
interface DebounceOptions {
  leading?: boolean | undefined;
  maxWait?: number | undefined;
  trailing?: boolean | undefined;
}

/** Functions to manage a debounced callback. */
type ControlFunctions = {
  /** Cancels pending function invocations. */
  cancel: () => void;
  /** Immediately invokes pending function invocations. */
  flush: () => void;
  /**
   * Checks if there are any pending function invocations.
   * @returns `true` if there are pending invocations, otherwise `false`.
   */
};

/**
 * Represents the state and control functions of a debounced callback.
 * Subsequent calls to the debounced function return the result of the last invocation.
 * Note: If there are no previous invocations, the result will be undefined.
 * Ensure proper handling in your code.
 */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type DebouncedState<T extends (...args: any) => ReturnType<T>> = ((
  ...args: Parameters<T>
) => ReturnType<T> | undefined) &
  ControlFunctions;

// eslint-disable-next-line @typescript-eslint/no-explicit-any
export const useDebouncedCallback = <T extends (...args: any) => ReturnType<T>>(
  fn: T,
  wait: number,
  settings: DebounceOptions = {
    leading: false,
    maxWait: undefined,
    trailing: true,
  },
): DebouncedState<T> => {
  const stable = useStableCallback(fn);
  const debounced = React.useMemo(
    () =>
      debounce(stable, wait, {
        leading: settings.leading,
        maxWait: settings.maxWait,
      }),
    [settings.leading, settings.maxWait, stable, wait],
  );

  React.useEffect(() => {
    /**
     * On unmount, flush the debounced callback to ensure it is called
     */
    return () => {
      if (settings.trailing) {
        debounced.flush();
      }
    };
  }, [debounced, settings.trailing]);

  return debounced;
};
