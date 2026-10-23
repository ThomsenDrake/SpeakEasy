"use client";

import * as SliderPrimitive from "@radix-ui/react-slider";
import type { VariantProps } from "class-variance-authority";
import * as React from "react";
import { tv } from "tailwind-variants";

import { cn } from "#utils/cn.ts";

export const sliderVariants = tv({
  slots: {
    root: ["relative flex w-full touch-none items-center select-none"],
    track: [
      "relative h-2 w-full grow cursor-pointer overflow-hidden rounded-full",
    ],
    range: ["absolute h-full"],
    thumb: [
      "bg-default ring-offset-background focus-visible:ring-default",
      "block h-5 w-5 rounded-full border-2 transition-colors",
      "focus-visible:ring-2 focus-visible:ring-offset-2 focus-visible:outline-hidden",
      "disabled:pointer-events-none disabled:opacity-50",
    ],
  },
  variants: {
    variant: {
      brand: {
        track: "bg-state-brand/10",
        range: "bg-state-brand",
        thumb: "border-orange",
      },
      muted: {
        track: "bg-basic-gray-alpha-10",
        range: "bg-basic-gray-accent",
        thumb: "bg-switch-handle",
      },
    },
  },
  defaultVariants: {
    variant: "brand",
  },
});

export type SliderVariantProps = VariantProps<typeof sliderVariants>;
export type SliderStyles = ReturnType<typeof sliderVariants>;

const Slider = ({
  ref,
  className,
  variant = "brand",
  ...props
}: React.ComponentProps<typeof SliderPrimitive.Root> & SliderVariantProps) => {
  const styles = sliderVariants({ variant });

  return (
    <SliderPrimitive.Root
      ref={ref}
      className={cn(styles.root(), className)}
      {...props}
    >
      <SliderPrimitive.Track
        data-slot="slider-track"
        className={styles.track()}
      >
        <SliderPrimitive.Range
          data-slot="slider-range"
          className={styles.range()}
        />
      </SliderPrimitive.Track>
      <SliderPrimitive.Thumb
        data-slot="slider-thumb"
        className={styles.thumb()}
      />
    </SliderPrimitive.Root>
  );
};
Slider.displayName = SliderPrimitive.Root.displayName;

export { Slider };
