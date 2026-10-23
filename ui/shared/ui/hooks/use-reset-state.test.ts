import { act, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { useResetState } from "./use-reset-state";

describe("useResetState", () => {
  it("should initialize with the initial state", () => {
    const initialState = "initial";
    const { result } = renderHook(() => useResetState(initialState, []));

    expect(result.current[0]).toBe(initialState);
  });

  it("should update the state", () => {
    const initialState = "initial";
    const { result } = renderHook(() => useResetState(initialState, []));

    act(() => {
      result.current[1]("updated");
    });

    expect(result.current[0]).toBe("updated");
  });

  it("should reset the state when dependencies change", () => {
    const initialState = "initial";
    const newInitialState = "new initial";
    const { result, rerender } = renderHook(
      (props) => useResetState(props.initialState, [props.initialState]),
      {
        initialProps: { initialState },
      },
    );

    act(() => {
      result.current[1]("updated");
    });

    expect(result.current[0]).toBe("updated");

    rerender({ initialState: newInitialState });

    expect(result.current[0]).toBe(newInitialState);
  });

  it("should handle function initial state", () => {
    const initialState = () => "initial";
    const { result } = renderHook(() => useResetState(initialState, []));

    expect(result.current[0]).toBe(initialState());
  });

  it("should handle function updates", () => {
    const initialState = "initial";
    const { result } = renderHook(() => useResetState(initialState, []));

    act(() => {
      result.current[1]((prevState) => `updated ${prevState}`);
    });

    expect(result.current[0]).toBe("updated initial");
  });

  it("should not reset the state when dependencies change but no new initial state is provided", () => {
    const initialState = "initial";
    const { result, rerender } = renderHook(
      (props) => useResetState(props.initialState, [props.initialState]),
      {
        initialProps: { initialState },
      },
    );

    act(() => {
      result.current[1]("updated");
    });

    expect(result.current[0]).toBe("updated");

    rerender({ initialState });

    expect(result.current[0]).toBe("updated");
  });

  it("should provide the latest value in setState, even if it's the external state", () => {
    const initialState = "initial";
    const { result, rerender } = renderHook(
      (props) => useResetState(props.initialState, [props.initialState]),
      {
        initialProps: { initialState },
      },
    );

    act(() => {
      result.current[1]((prevState) => `updated(${prevState})`);
    });

    expect(result.current[0]).toBe("updated(initial)");

    rerender({ initialState: "new external state" });

    act(() => {
      result.current[1]((prevState) => `updated(${prevState})`);
    });

    expect(result.current[0]).toBe("updated(new external state)");
  });
});
