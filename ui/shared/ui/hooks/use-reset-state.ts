import type { Dispatch, SetStateAction } from "react";
import React from "react";

import type { Option } from "../_stubs/option";
import { none, some } from "../_stubs/option";

/**
 * Use reset state allows holding some local state
 * in a component that can be synchronously reset to
 * its intial value when some dependencies change.
 *
 * It's useful when you want a component to be both
 * controlled and uncontrolled.
 *
 * @example
 * const [state, setState] = useResetState(stateFromProps, [stateFromProps]);
 *
 * // state will be reset to stateFromProps when stateFromProps changes
 *
 * setState("new value");
 *
 * // state will be "new value" until stateFromProps changes
 */
export const useResetState = <T>(
  initialState: T | (() => T),
  dependencies: unknown[],
): [T, Dispatch<SetStateAction<T>>] => {
  const [localState, setLocalState] = useForceUpdateState<T>(initialState);

  const externalStateRef = React.useRef<Option<T>>(none());

  React.useMemo(() => {
    externalStateRef.current = some(getState(initialState));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, dependencies);

  const setState = React.useCallback(
    (newState: SetStateAction<T>) => {
      setLocalState((localState) => {
        const latestState = externalStateRef.current
          ? externalStateRef.current.value
          : localState;

        externalStateRef.current = none();
        return apply(newState, latestState);
      });
    },
    [setLocalState],
  );

  return [
    externalStateRef.current ? externalStateRef.current.value : localState,
    setState,
  ];
};

/**
 * Like useState, but always re-render
 * the component when the state is updated, even when
 * it's equal to the previous state.
 */
const useForceUpdateState = <T>(
  initialState: T | (() => T),
): [T, Dispatch<SetStateAction<T>>] => {
  const [{ value }, setStateInternal] = React.useState<{ value: T }>(() => ({
    value: getState(initialState),
  }));
  const setState = React.useCallback((newState: SetStateAction<T>) => {
    setStateInternal(({ value }) => ({ value: apply(newState, value) }));
  }, []);
  return [value, setState];
};

/**
 * Helpers
 */

const apply = <T>(newState: SetStateAction<T>, oldState: T): T => {
  if (typeof newState === "function") {
    return (newState as (prevState: T) => T)(oldState);
  }
  return newState;
};

const getState = <T>(state: T | (() => T)): T => {
  if (typeof state === "function") {
    return (state as () => T)();
  }
  return state;
};
