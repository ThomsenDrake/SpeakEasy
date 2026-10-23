import { useEffect, useRef } from "react";

/**
 * Handles autofocus behavior for the input component. This is a custom hook that
 * ensures the autofocus behavior puts the cursor at the end of the input value.
 *
 * @param inputRef the input ref
 * @param autoFocus whether the input should be auto focused
 *
 * @example
 * ```tsx
 * const inputRef = useRef<HTMLInputElement>(null);
 * useInputAutofocusHandler(inputRef, autoFocus);
 *
 * return <input ref={inputRef} autoFocus={autoFocus} />
 * ```
 */
export function useInputAutofocusHandler(
  inputRef: React.RefObject<HTMLInputElement | null>,
  autoFocus?: boolean,
) {
  const didAutoFocus = useRef(false);

  useEffect(() => {
    if (inputRef.current && autoFocus) {
      inputRef.current.focus();
      // Move cursor to end only on automatic focus
      const { length } = inputRef.current.value;
      inputRef.current.setSelectionRange(length, length);
      didAutoFocus.current = true;
    }
  }, [autoFocus, inputRef]);
}
