import type { VariantProps } from "class-variance-authority";
import { tv } from "tailwind-variants";

export const typographyVariants = tv({
  base: "transition-colors [p,div]:whitespace-pre-line",
  variants: {
    variant: {
      default: "text-default",
      muted: "text-muted",
      subtle: "text-subtle",
      hint: "text-hint",
      inverted: "text-inverted-default",
      placeholder: "text-placeholder",
      destructive: "text-destructive",
    },
    size: {
      "2xs": "text-2xs leading-[0.75rem]",
      xs: "text-xs leading-[1rem]",
      sm: "text-sm leading-[1.25rem]",
      default: "text-base leading-[1.5rem]",
      lg: "text-lg leading-[1.75rem]",
      xl: "text-xl leading-[1.75rem]",
      "2xl": "text-2xl leading-[2rem]",
      "3xl": "text-3xl leading-[2.25rem]",
      "4xl": "text-4xl leading-[2.5rem]",
      "5xl": "text-5xl leading-[2.75rem]",
    },
    weight: {
      light: "font-light",
      normal: "font-normal",
      medium: "font-medium",
      semibold: "font-semibold",
      bold: "font-bold",
    },
  },
  compoundVariants: [],
  defaultVariants: {},
});

export type TypographyVariantProps = VariantProps<typeof typographyVariants>;
export type TypographyStyles = ReturnType<typeof typographyVariants>;
