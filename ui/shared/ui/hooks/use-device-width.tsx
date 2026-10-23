import { useWindowSize } from "usehooks-ts";

type DeviceType = "mobile" | "tablet" | "desktop";
// Match tailwind breakpoints
// https://tailwindcss.com/docs/responsive-design#overview
const MOBILE_BREAKPOINT = 768;
const DESKTOP_BREAKPOINT = 1280;

export function useDeviceType(): DeviceType {
  const { width } = useWindowSize();
  return width < MOBILE_BREAKPOINT
    ? "mobile"
    : width < DESKTOP_BREAKPOINT
      ? "tablet"
      : "desktop";
}
