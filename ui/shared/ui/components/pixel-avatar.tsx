"use client";

import { motion } from "framer-motion";
import { useEffect, useRef } from "react";

type PixelAvatarPattern = Array<Array<string | null>>;

interface PixelAvatarProps {
  data: PixelAvatarPattern;
  size?: number;
  animation?: "hovering" | "entry" | null;
}

export function PixelAvatar({
  data,
  size = 320,
  animation = null,
}: PixelAvatarProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    // Set the actual canvas dimensions to match your data
    canvas.width = size;
    canvas.height = size;

    const ctx = canvas.getContext("2d", { alpha: true });
    if (!ctx) return;

    // Disable image smoothing on the context
    ctx.imageSmoothingEnabled = false;

    // Clear the canvas
    ctx.clearRect(0, 0, canvas.width, canvas.height);

    // Calculate the correct scale (pixels per cell)
    const scale = size / data.length;

    // Draw each cell at the correct position
    for (const [y, row] of data.entries()) {
      for (const [x, cell] of row.entries()) {
        if (cell) {
          ctx.fillStyle = cell;
          // Use Math.round to ensure integer coordinates
          ctx.fillRect(
            Math.round(x * scale),
            Math.round(y * scale),
            Math.round(scale),
            Math.round(scale),
          );
        }
      }
    }
  }, [data, size]);

  return (
    <motion.canvas
      {...(animation === "entry"
        ? {
            initial: { opacity: 0, y: 8 },
            animate: { opacity: 1, y: 0 },
          }
        : animation === "hovering"
          ? {
              initial: { y: -1 },
              animate: { y: 1 },
              transition: {
                repeat: Infinity,
                repeatType: "reverse",
                ease: "easeInOut",
                duration: 0.75,
              },
            }
          : {})}
      ref={canvasRef}
      width={size}
      height={size}
      style={{
        imageRendering: "pixelated",
        padding: "10%",
      }}
    />
  );
}
