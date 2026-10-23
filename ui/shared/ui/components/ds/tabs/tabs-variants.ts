import type { VariantProps } from "class-variance-authority";
import { tv } from "tailwind-variants";

export const tabsVariants = tv({
  slots: {
    root: "flex flex-col w-full",
    list: "flex gap-4 w-fit border-b border-default",
    trigger: [
      "grow relative inline-flex items-center justify-center border-none",
      "font-medium text-muted transition-colors z-1 hover:text-default",
      "focus-visible:outline-none focus-visible:ring-default focus-visible:ring-2 focus-visible:ring-offset-2",
      "data-[state=active]:text-default",
    ],
    label: "relative z-1 text-xs sm:text-sm",
    indicator: "absolute z-0",
    content: [
      "w-full ring-offset-background",
      "focus-visible:ring-default focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-offset-2",
    ],
  },
  variants: {
    variant: {
      underline: {
        list: "border-b border-default",
        trigger: "h-12 px-0.5 py-[14px] rounded-xs",
        indicator: "h-[1.5px] -bottom-px w-full bg-state-primary",
      },
      pill: {
        list: "border-none",
        trigger: "h-8 px-2.5 py-1.5 rounded-xl",
        indicator: "inset-0 bg-state-soft rounded-xl",
      },
      segmented: {
        list: "bg-state-soft border-none h-9 p-0.5 rounded-sm",
        trigger: "h-8 px-2.5 py-1.5 rounded-sm",
        indicator: "inset-0 bg-card rounded-sm",
      },
      primary: {
        list: "bg-card border-none h-9 p-0.5 rounded-2xl",
        trigger:
          "h-8 px-2.5 py-1.5 rounded-xl data-[state=active]:text-inverted-default data-[state=active]:bg-state-primary",
        indicator: "inset-0 bg-state-primary rounded-xl",
      },
      circled: {
        list: "border-none",
        trigger:
          "p-2 rounded-md data-[state=active]:border-orange data-[state=active]:bg-default",
        indicator: "inset-0 rounded-md border-2 border-orange",
      },
      left: {
        list: "border-none font-normal",
        trigger:
          "h-12 px-8 py-[14px] rounded-xs data-[state=active]:bg-state-soft",
        indicator: "w-[1.5px] -left-px h-full bg-basic-orange-accent",
      },
    },
  },
  defaultVariants: {
    variant: "underline",
  },
});

export type TabsVariantProps = VariantProps<typeof tabsVariants>;
export type TabsStyles = ReturnType<typeof tabsVariants>;
