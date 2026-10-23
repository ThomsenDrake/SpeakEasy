import React from "react";

/**
 * Returns an up-to-date ref to a value.
 * Useful when you want to access some state
 * in an effect without re-triggering it when
 * this state changes.
 */
export const useFollowingRef = <T>(value: T): React.RefObject<T> => {
  const ref = React.useRef(value);
  React.useLayoutEffect(() => {
    ref.current = value;
  });
  return ref;
};
