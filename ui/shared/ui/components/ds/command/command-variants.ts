import type { VariantProps } from "class-variance-authority";
import { tv } from "tailwind-variants";

export const commandVariants = tv({
  slots: {
    command:
      "bg-default text-default flex h-full w-full flex-col overflow-hidden rounded-md",
    dialogCommandContent:
      "p-0 shadow-lg translate-y-0 top-1/6 max-h-[calc(5/6*100vh-4px)]",
    dialogCommand:
      "[&_[cmdk-group-heading]]:text-muted [&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:font-medium [&_[cmdk-group]:not([hidden])_~[cmdk-group]]:pt-0 [&_[cmdk-group]]:px-2 [&_[cmdk-input-wrapper]_svg]:h-5 [&_[cmdk-input-wrapper]_svg]:w-5 [&_[cmdk-input]]:h-12 [&_[cmdk-item]]:px-2 [&_[cmdk-item]]:py-3 [&_[cmdk-item]_svg]:h-5 [&_[cmdk-item]_svg]:w-5",
    inputContainer:
      "has-[input:focus]:ring-0 rounded-b-none border-t-0 border-s-0 border-e-0 has-[input:focus]:shadow-none bg-transparent",
    list: "overflow-y-auto overflow-x-hidden transition-all max-h-[calc(5/6*100vh-120px)] h-(--cmdk-list-height)",
    empty: "text-center text-sm text-subtle",
    group:
      "text-default [&_[cmdk-group-heading]]:text-muted overflow-hidden [&_[cmdk-group-heading]]:text-xs [&_[cmdk-group-heading]]:font-medium",
    item: "gap-2 text-subtle aria-selected:bg-state-soft aria-selected:text-default outline-hidden relative flex cursor-default select-none items-center rounded-sm text-sm data-[disabled=true]:pointer-events-none data-[disabled=true]:opacity-50",
    itemAsideIcon: "size-[1.25em]",
    separator: "border-default -mx-1 border",
    loading:
      "flex flex-row items-center justify-center gap-2 text-hint text-sm",
  },
  variants: {
    size: {
      default: {
        item: "px-2 py-1.5",
        group:
          "p-1 [&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:py-1.5",
        empty: "py-6",
      },
      lg: {
        item: "px-4 py-2",
        group:
          "p-1 [&_[cmdk-group-heading]]:px-4 [&_[cmdk-group-heading]]:py-2",
        empty: "py-8",
        inputContainer: "h-12",
      },
    },
  },
  defaultVariants: {},
});

export type CommandVariantProps = VariantProps<typeof commandVariants>;
export type CommandStyles = ReturnType<typeof commandVariants>;
