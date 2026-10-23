// Minimal Option monad — stub for @mistral/common/types/option + utils/option.
// Just enough to make use-reset-state.ts work.

export type Option<T> = { value: T; defined: true } | { defined: false };

export const some = <T>(value: T): Option<T> => ({ value, defined: true });
export const none = <T>(): Option<T> => ({ defined: false });
