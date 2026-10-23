import React from "react";

/**
 * Take a function and return a stable reference to it.
 * Captured scope variable will always be up to date.
 *
 * Warning: Only use for callbacks / event handlers and not for functions
 * rendering components. Since the function reference is stable, React may skip
 * re-rendering the component if one of its dependencies changes.
 *
 * If the callback is only called from inside a `useEffect`, prefer
 * `useEffectEvent` from React 19.2+ which serves the same purpose
 * with first-party semantics.
 */
export function useStableCallback<Args extends readonly unknown[], Return>(
  callback: (...args: Args) => Return,
): (...args: Args) => Return {
  const callbackRef = React.useRef(callback);
  React.useLayoutEffect(() => {
    callbackRef.current = callback;
  });
  return React.useCallback((...args: Args) => {
    return callbackRef.current(...args);
  }, []);
}
