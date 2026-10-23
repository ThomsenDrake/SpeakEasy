import { useIntersectionObserver } from "usehooks-ts";

export function useIntersectionObserverTopBottom(): {
  topSentinelRef: (node?: Element | null) => void;
  isTopIntersecting: boolean;
  bottomSentinelRef: (node?: Element | null) => void;
  isBottomIntersecting: boolean;
} {
  const { ref: topSentinelRef, isIntersecting: isTopIntersecting } =
    useIntersectionObserver({
      threshold: 0.5,
      initialIsIntersecting: true,
    });
  const { ref: bottomSentinelRef, isIntersecting: isBottomIntersecting } =
    useIntersectionObserver({
      threshold: 0.5,
      initialIsIntersecting: true,
    });

  return {
    topSentinelRef,
    isTopIntersecting,
    bottomSentinelRef,
    isBottomIntersecting,
  };
}
