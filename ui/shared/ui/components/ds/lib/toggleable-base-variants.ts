import type { VariantProps } from "class-variance-authority";
import { tv } from "tailwind-variants";

export const toggleableBaseVariants = tv({
  slots: {
    hiddenInput: "peer opacity-0 size-0",
    toggleable:
      "group inline has-aria-disabled:cursor-not-allowed has-aria-disabled:[&_*]:pointer-events-none",
    container: [
      "group/container flex items-start gap-2.5",
      "[&>label]:grow [&>label]:space-y-1",
      "has-aria-invalid:[&>label]:text-destructive has-aria-disabled:[&>label]:cursor-not-allowed",
    ],
    label: "",
    description: "",
  },
  compoundSlots: [
    {
      slots: ["label", "description"],
      className:
        "block cursor-pointer group-has-aria-disabled/container:cursor-not-allowed",
    },
  ],
  variants: {
    position: {
      left: {},
      right: {
        container: "flex-row-reverse",
      },
    },
  },
  compoundVariants: [],
  defaultVariants: {
    position: "left",
  },
});

export type ToggleableVariantProps = VariantProps<
  typeof toggleableBaseVariants
>;
export type ToggleableStyles = ReturnType<typeof toggleableBaseVariants>;
