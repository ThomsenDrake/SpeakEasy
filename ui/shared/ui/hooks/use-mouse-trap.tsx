import type { RefObject } from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

const DEFAULT_MOUSE_TRAP_INACTIVITY_MS = 20;

export const MOUSE_TRAP_DEBUG = Symbol("DEBUG");

interface Point {
  x: number;
  y: number;
}

interface UseMouseTrapOptions {
  containerRef: RefObject<HTMLElement | null>;
  enabled?: boolean;
  inactivityMs?: number;
  [MOUSE_TRAP_DEBUG]?: boolean;
}

interface UseMouseTrapResult {
  mouseTrapElement: React.ReactNode | null;
}

/**
 * Tracks mouse movement around an element and returns polygon points that can be
 * used to render a mouse trap between the pointer and the element edge.
 * This is useful for things like submenus or tooltips.
 */
export function useMouseTrap({
  containerRef,
  enabled = true,
  inactivityMs = DEFAULT_MOUSE_TRAP_INACTIVITY_MS,
  [MOUSE_TRAP_DEBUG]: debug = false,
}: UseMouseTrapOptions): UseMouseTrapResult {
  // References management
  const hideMouseTrapTimeoutRef = useRef<number | null>(null);
  const animationFrameRequestIdRef = useRef<number | null>(null);

  // Coordinates state
  const [mousePosition, setMousePosition] = useState<Point | null>(null);
  const [trapBounds, setTrapBounds] = useState<DOMRect | null>(null);

  // Visibility state
  const [isMouseTrapVisible, setIsMouseTrapVisible] = useState(false);

  /** Cancel callbacks (timeout + animation frame) */
  const cancelScheduledCallbacks = useCallback(() => {
    if (hideMouseTrapTimeoutRef.current) {
      window.clearTimeout(hideMouseTrapTimeoutRef.current);
      hideMouseTrapTimeoutRef.current = null;
    }
    if (animationFrameRequestIdRef.current !== null) {
      cancelAnimationFrame(animationFrameRequestIdRef.current);
      animationFrameRequestIdRef.current = null;
    }
  }, []);

  /** Hide mouse trap */
  const hideMouseTrap = useCallback(() => {
    setIsMouseTrapVisible(false);
    setTrapBounds(null);
  }, []);

  // Schedule mouse trap hide management
  const scheduleMouseTrapHide = useCallback((): void => {
    cancelScheduledCallbacks();
    hideMouseTrapTimeoutRef.current = window.setTimeout(() => {
      hideMouseTrap();
    }, inactivityMs);
  }, [cancelScheduledCallbacks, hideMouseTrap, inactivityMs]);

  // Mouse event management
  useEffect(() => {
    if (!enabled) {
      cancelScheduledCallbacks();
      hideMouseTrap();
      return;
    }

    const handleMouseMove = (event: MouseEvent) => {
      cancelScheduledCallbacks();
      animationFrameRequestIdRef.current = requestAnimationFrame(() => {
        const bounds = containerRef.current?.getBoundingClientRect();
        if (!bounds) {
          hideMouseTrap();
          cancelScheduledCallbacks();
          return;
        }

        const x = event.clientX;
        const y = event.clientY;

        const isMouseInsideContainer =
          x >= bounds.left &&
          x <= bounds.right &&
          y >= bounds.top &&
          y <= bounds.bottom;

        if (isMouseInsideContainer) {
          hideMouseTrap();
          cancelScheduledCallbacks();
          return;
        }

        setMousePosition({ x, y });
        setTrapBounds(bounds);
        setIsMouseTrapVisible(true);
        scheduleMouseTrapHide();
      });
    };

    window.addEventListener("mousemove", handleMouseMove, { passive: true });

    return () => {
      window.removeEventListener("mousemove", handleMouseMove);
      cancelScheduledCallbacks();
    };
  }, [
    cancelScheduledCallbacks,
    containerRef,
    enabled,
    hideMouseTrap,
    scheduleMouseTrapHide,
  ]);

  // Debug management
  useEffect(() => {
    if (debug && containerRef.current) {
      containerRef.current.style.border = "1px solid rgba(255, 120, 0, 0.5)";
    }
  }, [debug, containerRef]);

  // Element coordinates computation
  const mouseTrapPoints = useMemo(
    () =>
      enabled && isMouseTrapVisible && mousePosition && trapBounds
        ? computeMouseTrapPoints(mousePosition, trapBounds)
        : null,
    [enabled, isMouseTrapVisible, mousePosition, trapBounds],
  );

  /** Mouse trap element, rendered as a SVG polygon */
  const mouseTrapElement = useMemo(
    () =>
      mouseTrapPoints ? (
        <svg
          aria-hidden
          className="pointer-events-none fixed inset-0 z-50 cursor-pointer"
          width="100%"
          height="100%"
        >
          <polygon
            points={mouseTrapPoints}
            className="pointer-events-auto"
            {...(debug
              ? { fill: "rgba(255, 0, 0, 0.2)", stroke: "rgba(255, 0, 0, 0.5)" }
              : { fill: "transparent" })}
          />
        </svg>
      ) : null,
    [mouseTrapPoints, debug],
  );

  return { mouseTrapElement };
}

type Coords = `${number},${number}`;

function computeMouseTrapPoints(
  { x, y }: Point,
  { left, right, top, bottom }: DOMRect,
): `${Coords} ${Coords} ${Coords}` | null {
  // Compute mouse position relative to the trap bounds
  const isLeft = x <= left;
  const isRight = x >= right;
  const isAbove = y <= top;
  const isBelow = y >= bottom;

  // Parse mouse position
  const mouse: Coords = `${x},${y}`;

  // Compute coordinates of the triangle corners
  /**
   *   🐁                              🐁
   *      --------------------------
   *      |                        |
   *      |                        |
   *      |                        |
   *      --------------------------
   *   🐁                              🐁
   */
  if ((isLeft && isAbove) || (isRight && isBelow))
    return `${mouse} ${right},${top} ${left},${bottom}`;
  if ((isRight && isAbove) || (isLeft && isBelow))
    return `${mouse} ${left},${top} ${right},${bottom}`;

  /**
   *                  🐁
   *      --------------------------
   *      |                        |
   *  🐁  |                        |  🐁
   *      |                        |
   *      --------------------------
   *                  🐁
   */
  if (isLeft) return `${mouse} ${left},${top} ${left},${bottom}`;
  if (isRight) return `${mouse} ${right},${top} ${right},${bottom}`;
  if (isAbove) return `${mouse} ${left},${top} ${right},${top}`;
  if (isBelow) return `${mouse} ${left},${bottom} ${right},${bottom}`;

  return null;
}
