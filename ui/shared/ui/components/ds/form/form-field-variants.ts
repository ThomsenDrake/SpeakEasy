import type { VariantProps } from "class-variance-authority";
import { tv } from "tailwind-variants";

import { typographyVariants } from "../typography/typography-variants";

export const formFieldVariants = tv({
  slots: {
    container: "",
    label: typographyVariants({
      className: "block mb-2",
      size: "sm",
      weight: "medium",
    }),
    hint: typographyVariants({
      className: "pt-1",
      size: "xs",
      variant: "muted",
    }),
  },
  compoundSlots: [],
  variants: {},
  compoundVariants: [],
  defaultVariants: {},
});

export type FormFieldVariantProps = VariantProps<typeof formFieldVariants>;
export type FormFieldStyles = ReturnType<typeof formFieldVariants>;
