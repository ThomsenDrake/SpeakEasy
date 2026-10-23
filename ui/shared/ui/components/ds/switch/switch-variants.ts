import type { VariantProps } from "class-variance-authority";
import { tv } from "tailwind-variants";

import { toggleableBaseVariants } from "../lib/toggleable-base-variants";

export const switchVariants = tv({
  extend: toggleableBaseVariants,
  slots: {
    hiddenInput: "",
    container: "",
    label: "",
    description: "",
    track: [
      "h-5 w-8 px-[3px] inline-flex shrink-0 items-center rounded-full transition-all",
      "cursor-pointer peer-aria-disabled:cursor-not-allowed",
      "peer-focus-visible:ring-3 peer-focus-visible:ring-offset-1 peer-focus-visible:ring-default outline-hidden",
      "bg-switch-default peer-aria-checked:bg-switch-active",
      "group-hover:bg-switch-default-hover group-hover:peer-aria-checked:bg-switch-active-hover",
      "group-hover/container:bg-switch-default-hover group-hover/container:peer-aria-checked:bg-switch-active-hover",
      "peer-aria-disabled:bg-switch-disabled peer-aria-disabled:peer-aria-checked:bg-switch-active-disabled",
    ],
    thumb: [
      "size-3.5 pointer-events-none bg-default block rounded-full shadow-switch-handle transition-transform",
      "translate-x-0 group-has-aria-checked:translate-x-3",
    ],
  },
  compoundSlots: [
    {
      slots: ["label", "description"],
      className:
        "block cursor-pointer group-has-aria-disabled/container:cursor-not-allowed",
    },
  ],
  variants: {},
  compoundVariants: [],
  defaultVariants: {
    position: "left",
  },
});

export type SwitchVariantProps = VariantProps<typeof switchVariants>;
export type SwitchStyles = ReturnType<typeof switchVariants>;
