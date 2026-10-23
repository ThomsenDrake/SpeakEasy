import type { VariantProps } from "class-variance-authority";
import { tv } from "tailwind-variants";

export const inlineTipVariants = tv({
  slots: {
    line: "w-1 rounded-full",
    container:
      "py-2.5 px-3 text-sm rounded-md text-subtle grid grid-cols-[4px_1fr] text-start gap-3",
  },
  variants: {
    state: {
      default: {
        line: "bg-basic-gray-accent",
        container: "bg-basic-gray-subtle",
      },
      information: {
        line: "bg-basic-blue-accent",
        container: "bg-basic-blue-subtle",
      },
      success: {
        line: "bg-basic-green-accent",
        container: "bg-basic-green-subtle",
      },
      warning: {
        line: "bg-basic-orange-accent",
        container: "bg-basic-orange-subtle",
      },
      error: {
        line: "bg-basic-red-accent",
        container: "bg-basic-red-subtle",
      },
    },
  },
  compoundVariants: [],
  defaultVariants: {
    state: "default",
  },
});

export type InlineTipVariantProps = VariantProps<typeof inlineTipVariants>;
