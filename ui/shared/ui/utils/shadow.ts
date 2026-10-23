export const insetBoxShadowStyle = function (
  shadowTop: boolean,
  shadowBottom: boolean,
): string {
  return shadowTop && shadowBottom
    ? "inset 0 6px 6px -6px rgba(0,0,0,0.1), inset 0 -6px 6px -6px rgba(0,0,0,0.1) "
    : shadowTop
      ? "inset 0 6px 6px -6px rgba(0,0,0,0.1)"
      : shadowBottom
        ? "inset 0 -6px 6px -6px rgba(0,0,0,0.1)"
        : "none";
};
