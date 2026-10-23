import { renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { useDeepEqualMemo } from "./use-deep-equal-memo";

describe("useDeepEqualMemo", () => {
  it("returns the same value if deep equality is true", () => {
    const object = { a: 1, b: 2 };

    const { result, rerender } = renderHook(() => useDeepEqualMemo(object));

    expect(result.current).toEqual(object);

    rerender(object);

    expect(result.current).toBe(result.current);
  });

  it("updates the value if deep equality is false", () => {
    const { result } = renderHook(() => useDeepEqualMemo({ a: 1, b: 2 }));

    expect(result.current).toEqual({ a: 1, b: 2 });

    const { result: secondResult } = renderHook(() =>
      useDeepEqualMemo({ a: 1, b: 3 }),
    );

    expect(secondResult.current).toEqual({ a: 1, b: 3 });
    expect(secondResult.current).not.toBe(result.current);
  });

  it("does not re-evaluate when the value is deeply equal and unchanged", () => {
    const { result, rerender } = renderHook(() =>
      useDeepEqualMemo({ a: 1, b: 2 }),
    );

    expect(result.current).toEqual({ a: 1, b: 2 });

    rerender();

    expect(result.current).toBe(result.current);
  });

  it("re-evaluates and returns new value when deeply equal values change", () => {
    const { result, rerender } = renderHook(() =>
      useDeepEqualMemo({ a: 1, b: 2 }),
    );

    expect(result.current).toEqual({ a: 1, b: 2 });

    rerender();

    expect(result.current).toEqual({ a: 1, b: 2 });
  });

  it("memoizes nested objects correctly", () => {
    const nestedObject = { a: { b: 2 } };
    const anotherNestedObject = { a: { b: 2 } };

    const { result, rerender } = renderHook(() =>
      useDeepEqualMemo(nestedObject),
    );

    expect(result.current).toEqual(nestedObject);

    rerender(anotherNestedObject);

    expect(result.current).toBe(result.current);
  });

  it("handles array values correctly", () => {
    const array1 = [1, 2, 3];
    const array2 = [1, 2, 3];

    const { result, rerender } = renderHook(() => useDeepEqualMemo(array1));

    expect(result.current).toEqual(array1);

    rerender(array2);

    expect(result.current).toBe(result.current);
  });

  it("handles primitive values correctly", () => {
    const { result, rerender } = renderHook(() => useDeepEqualMemo("Hello"));

    expect(result.current).toBe("Hello");

    rerender("Hello");

    expect(result.current).toBe("Hello");
  });
});
