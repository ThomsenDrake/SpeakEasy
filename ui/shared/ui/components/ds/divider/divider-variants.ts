import type { VariantProps } from "class-variance-authority";
import { tv } from "tailwind-variants";

export const dividerVariants = tv({
  slots: {
    container: "group flex items-center gap-2 [&>hr]:grow",
    base: [
      "border-default",
      "group-has-data-[position=start]:first-of-type:hidden group-has-data-[position=end]:last-of-type:hidden",
    ],
    label: "peer text-muted text-sm",
  },
  variants: {
    variant: {
      dashed: {
        base: "border-dashed",
      },
    },
  },
  compoundVariants: [],
  defaultVariants: {},
});

export type DividerVariantProps = VariantProps<typeof dividerVariants>;
