import type { VariantProps } from "class-variance-authority";
import type { Variants } from "framer-motion";
import { tv } from "tailwind-variants";

import { toggleableBaseVariants } from "../lib/toggleable-base-variants";

export const checkboxVariants = tv({
  extend: toggleableBaseVariants,
  slots: {
    hiddenInput: "",
    container: "[&>:first-child]:translate-y-[2px]",
    icon: "*:size-5",
    label: "",
    description: "",
    toggleable:
      "inline-flex has-focus-visible:ring-3 has-focus-visible:ring-offset-1 has-focus-visible:ring-default outline-hidden overflow-visible rounded-[5px]",
    checkbox: [
      "flex items-center justify-center size-4 border bg-default inline-flex rounded-[5px] transition-all cursor-pointer",
      "group-has-aria-checked:hover:bg-checkbox-active-hover!",
      "group-has-aria-[checked='false']:hover:border-strong!",
      "group-has-aria-[checked='false']:group-hover/container:border-strong!",
    ],
    ticks: "size-3 stroke-[3.5]",
    phantomTick: [
      "group-has-aria-checked:invisible! opacity-0 group-hover:opacity-100 transition-opacity",
      // todo understand why this glitches when unchecking the checkbox from its label
      // "group-has-aria-[checked='false']:group-hover/container:opacity-100",
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
        checkbox: "inset-shadow-checkbox-active",
      },
    },
  },
  compoundVariants: [],
  defaultVariants: {
    position: "left",
    variant: "default",
  },
});

export type CheckboxVariantProps = VariantProps<typeof checkboxVariants>;
export type CheckboxStyles = ReturnType<typeof checkboxVariants>;

export const checkboxMotion = {
  box: {
    checked: {
      backgroundColor: "var(--bg-checkbox-active)",
      borderColor: "var(--bg-checkbox-active)",
      transition: { duration: 0.03 },
    },
    unchecked: {
      backgroundColor: "var(--bg-checkbox-default)",
      borderColor: "var(--border-default)",
      transition: { duration: 0.03, delay: 0.01 },
    },
  },
  tick: {
    checked: {
      pathLength: 1,
      stroke: "var(--text-inverted-default)",
      scale: 1,
      transition: { duration: 0.06, delay: 0.02 },
    },
    unchecked: {
      pathLength: 0,
      stroke: "var(--transparent-light-00)",
      scale: 0.75,
      transition: { duration: 0.04 },
    },
  },
  phantomTick: {
    checked: {
      stroke: "var(--text-inverted-default)",
      transition: { duration: 0.07, delay: 0.04 },
    },
    unchecked: {
      stroke: "var(--icon-default-disabled)",
      transition: { duration: 0.07, delay: 0.03 },
    },
  },
} satisfies Record<string, Variants>;
