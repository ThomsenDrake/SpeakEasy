import type { VariantProps } from "class-variance-authority";
import { tv } from "tailwind-variants";

import { toggleableBaseVariants } from "../lib/toggleable-base-variants";

export const radioVariants = tv({
  extend: toggleableBaseVariants,
  slots: {
    group: "space-y-2",
    hiddenInput: "",
    container: "[&>:first-child]:translate-y-[2px]",
    label: "",
    description: "",
    toggleable:
      "inline-flex has-focus-visible:ring-3 has-focus-visible:ring-offset-1 has-focus-visible:ring-default outline-hidden overflow-visible rounded-full",
    radio: [
      "relative flex items-center justify-center size-4 border border-transparent bg-default inline-flex rounded-full transition-all cursor-pointer",
      "group-has-aria-checked:bg-checkbox-active group-has-aria-checked:hover:bg-checkbox-active-hover",
      "group-has-aria-[checked=false]:border-default group-has-aria-[checked=false]:hover:border-strong",
      "group-has-aria-[checked=false]:group-hover/container:border-strong!",
      "group-has-aria-disabled:bg-checkbox-disabled group-has-aria-disabled:border-default",
    ],
    innerCircle: [
      "absolute inset-0 bg-default rounded-full",
      "group-has-aria-checked:scale-50 transition-transform",
      "group-has-[input[type='radio']:not(:checked):disabled]:bg-checkbox-disabled",
      "group-has-[input[type='radio']:checked:disabled]:bg-(--icon-default-disabled)",
    ],
    phantomCircle: [
      "absolute inset-0 group-has-aria-checked:invisible! scale-0 group-hover:scale-50 transition-transform rounded-full bg-muted",
    ],
  },
  compoundSlots: [
    {
      slots: ["label", "description"],
      className:
        "block cursor-pointer group-has-aria-disabled/container:cursor-not-allowed",
    },
  ],
  variants: {
    variant: {
      default: {},
      "with-shadow": {
        radio: "inset-shadow-checkbox-active",
      },
    },
  },
  compoundVariants: [],
  defaultVariants: {
    position: "left",
    variant: "default",
  },
});

export type RadioVariantProps = VariantProps<typeof radioVariants>;
export type RadioStyles = ReturnType<typeof radioVariants>;
