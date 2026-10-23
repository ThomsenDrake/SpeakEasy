import type { VariantProps } from "class-variance-authority";
import { tv } from "tailwind-variants";

export const badgeVariants = tv({
  slots: {
    base: "font-medium flex items-center gap-1 whitespace-nowrap",
    icon: "m-[1px]",
    dot: "",
  },
  variants: {
    size: {
      xs: {
        base: "text-[10px] px-1 h-4",
        icon: "size-2.5",
        dot: "ml-[1px] mr-0.5 size-1",
      },
      sm: {
        base: "text-xs px-1.5 h-5",
        icon: "size-3",
        dot: "ml-[1px] mr-0.5 size-1.5",
      },
      lg: {
        base: "text-xs px-2 h-6",
        icon: "size-3",
        dot: "mr-0.5 size-2",
      },
      xl: {
        base: "text-sm px-2.5 h-7",
        icon: "size-3.5",
        dot: "mr-0.5 size-2.5",
      },
      "2xl": {
        base: "text-sm px-3 h-8",
        icon: "size-4",
        dot: "mr-0.5 size-3",
      },
    },
    variant: {
      neutral: {
        base: "bg-badge-default text-subtle",
        icon: "text-icon-default-muted",
      },
      red: {
        dot: "text-(--bg-basic-red-accent)",
        base: "bg-badge-red text-(--bg-basic-red-strong)",
      },
      orange: {
        dot: "text-(--bg-basic-orange-accent)",
        base: "bg-badge-orange text-(--bg-basic-orange-strong)",
      },
      orangebright: {
        dot: "text-(--bg-basic-orangebright-accent)",
        base: "bg-badge-orangebright text-(--bg-basic-orangebright-strong)",
      },
      yellow: {
        dot: "text-(--bg-basic-yellow-accent)",
        base: "bg-badge-yellow text-(--bg-basic-yellow-strong)",
      },
      lime: {
        dot: "text-(--bg-basic-lime-accent)",
        base: "bg-badge-lime text-(--bg-basic-lime-strong)",
      },
      green: {
        dot: "text-(--bg-basic-green-accent)",
        base: "bg-badge-green text-(--bg-basic-green-contrast)",
      },
      emerald: {
        dot: "text-(--bg-basic-emerald-accent)",
        base: "bg-badge-emerald text-(--bg-basic-emerald-strong)",
      },
      teal: {
        dot: "text-(--bg-basic-teal-accent)",
        base: "bg-badge-teal text-(--bg-basic-teal-strong)",
      },
      cyan: {
        dot: "text-(--bg-basic-cyan-accent)",
        base: "bg-badge-cyan text-(--bg-basic-cyan-strong)",
      },
      sky: {
        dot: "text-(--bg-basic-sky-accent)",
        base: "bg-badge-sky text-(--bg-basic-sky-strong)",
      },
      blue: {
        dot: "text-(--bg-basic-blue-accent)",
        base: "bg-badge-blue text-(--bg-basic-blue-strong)",
      },
      bluedeep: {
        dot: "text-(--bg-basic-bluedeep-accent)",
        base: "bg-badge-bluedeep text-(--bg-basic-bluedeep-strong)",
      },
      purpledeep: {
        dot: "text-(--bg-basic-purpledeep-accent)",
        base: "bg-badge-purpledeep text-(--bg-basic-purpledeep-strong)",
      },
      purple: {
        dot: "text-(--bg-basic-purple-accent)",
        base: "bg-badge-purple text-(--bg-basic-purple-strong)",
      },
      pinkdeep: {
        dot: "text-(--bg-basic-pinkdeep-accent)",
        base: "bg-badge-pinkdeep text-(--bg-basic-pinkdeep-strong)",
      },
      pink: {
        dot: "text-(--bg-basic-pink-accent)",
        base: "bg-badge-pink text-(--bg-basic-pink-strong)",
      },
      rose: {
        dot: "text-(--bg-basic-rose-accent)",
        base: "bg-badge-rose text-(--bg-basic-rose-strong)",
      },
    },
    bordered: {
      true: "border border-default",
    },
    shape: {
      rounded: "rounded-sm",
      pill: "rounded-full",
    },
  },
  compoundVariants: [
    {
      bordered: false,
      variant: "neutral",
      class: "bg-badge-gray",
    },
  ],
  defaultVariants: {
    size: "lg",
    variant: "neutral",
    bordered: false,
    shape: "rounded",
  },
});

export type BadgeVariantProps = VariantProps<typeof badgeVariants>;
export type BadgeStyles = ReturnType<typeof badgeVariants>;
