import type { VariantProps } from "class-variance-authority";
import { tv } from "tailwind-variants";

export const buttonVariants = tv({
  slots: {
    group: [
      "flex",
      "[&>:not(:first-child)]:rounded-s-none [&>:not(:last-child)]:rounded-e-none",
      "[&>:not(:first-child)]:border-s-0",
    ],
    base: [
      "flex items-center justify-center text-center font-medium cursor-pointer",
      "outline-hidden focus-visible:ring-3 relative whitespace-nowrap transition-colors",
      "focus-visible:ring-default focus-visible:ring-offset-1",
      "aria-disabled:text-hint aria-disabled:cursor-not-allowed",
      "aria-busy:cursor-wait aria-busy:text-transparent",
      "aria-disabled:aria-busy:text-transparent",
    ],
    asideIcon: "in-aria-busy:text-transparent",
    // the DS is 20px but our icons have a bit less margin
    onlyIcon: "size-[18px]",
    spinner:
      "inset-0 flex items-center justify-center absolute text-icon-default aria-disabled:text-hint",
  },
  variants: {
    variant: {
      none: {},
      primary: {
        group:
          "[&>button]:not-first:border-s-1 [&>button]:not-first:border-inverted",
        base: "text-inverted-default bg-state-primary hover:bg-state-primary-hover active:bg-state-primary-press aria-busy:bg-state-primary-loading inset-shadow-components-default aria-disabled:bg-state-disabled",
        spinner: "text-icon-inverted-default",
      },
      brand: {
        group:
          "[&>button]:not-first:border-s-1 [&>button]:not-first:border-inverted",
        base: "text-white-default bg-state-brand hover:bg-state-brand-hover active:bg-state-brand-press aria-busy:bg-state-brand-loading inset-shadow-components-default aria-disabled:bg-state-disabled",
        spinner: "text-icon-white-default",
      },
      secondary: {
        base: "text-default bg-state-secondary hover:bg-state-secondary-hover active:bg-state-secondary-press aria-busy:bg-state-secondary-loading border-darker border-[0.5px] aria-disabled:text-muted",
        spinner: "text-icon-default-subtle",
      },
      dashed: {
        base: "text-default bg-state-secondary hover:bg-state-secondary-hover active:bg-state-secondary-press aria-busy:bg-state-secondary-loading border-darker border border-dashed aria-disabled:bg-state-disabled",
        spinner: "text-icon-default-subtle",
        asideIcon: "text-icon-default-muted",
      },
      soft: {
        group: "[&>button]:not-first:border-s-1",
        base: "bg-state-soft text-default hover:bg-state-soft-hover active:bg-state-soft-press aria-disabled:bg-state-disabled",
        spinner: "text-icon-default-muted",
      },
      ghost: {
        base: "bg-state-ghost text-default hover:not-disabled:bg-state-ghost-hover active:not-disabled:bg-state-ghost-press",
        spinner: "text-icon-default-muted",
      },
      "ghost-muted": {
        base: "bg-state-ghost text-muted hover:not-disabled:bg-state-ghost-hover active:not-disabled:bg-state-ghost-press",
        spinner: "text-icon-default-muted",
      },
      destructive: {
        base: "bg-state-destructive text-inverted-default hover:bg-state-destructive-hover active:bg-state-destructive-press aria-disabled:bg-state-disabled",
        spinner: "text-icon-inverted-default",
      },
    },
    size: {
      "2xs": {
        base: "h-6 gap-1 px-1.5 text-xs",
        asideIcon: "size-4",
        onlyIcon: "size-3.5", // the DS is 16px but our icons have a bit less margin
      },
      xs: {
        base: "h-7 gap-1 px-2 text-sm",
        asideIcon: "size-4",
      },
      sm: {
        base: "h-8 gap-1 px-2.5 text-sm",
        asideIcon: "size-4",
      },
      default: {
        base: "h-9 gap-1.5 px-3 text-sm",
        asideIcon: "size-4",
      },
      lg: {
        base: "h-10 gap-2 px-3.5 text-sm",
        asideIcon: "size-[18px]",
      },
    },
    mode: {
      default: "",
      "icon-only": "p-0",
    },
    shape: {
      rounded: "rounded-md",
      pill: "rounded-full",
    },
    inverted: {
      false: {},
      true: {},
    },
  },
  compoundVariants: [
    // `inverted` is currently defined only for the `soft` variant.
    // Other variants intentionally keep their default styles until the DS has
    // explicit inverted-surface specs for them.
    {
      variant: "soft",
      inverted: true,
      class: {
        base: "bg-(--transparent-light-8) text-white-default hover:bg-(--transparent-light-10) active:bg-(--transparent-light-15) dark:bg-(--transparent-dark-6) dark:text-dark-default dark:hover:bg-(--transparent-dark-8) dark:active:bg-(--transparent-dark-10)",
        spinner: "text-icon-white-subtle dark:text-icon-default-muted",
      },
    },
    {
      mode: "icon-only",
      size: "lg",
      class: { base: "size-10" },
    },
    {
      mode: "icon-only",
      size: "default",
      class: { base: "size-9" },
    },
    {
      mode: "icon-only",
      size: "sm",
      class: { base: "size-8" },
    },
    {
      mode: "icon-only",
      size: "xs",
      class: { base: "size-7" },
    },
    {
      mode: "icon-only",
      size: "2xs",
      class: { base: "size-6" },
    },
    {
      mode: "icon-only",
      variant: ["secondary", "dashed", "soft"],
      class: "text-icon-default-subtle",
    },
    {
      mode: "icon-only",
      variant: "soft",
      inverted: true,
      class: "text-icon-white-subtle dark:text-icon-default-subtle",
    },
    {
      mode: "icon-only",
      variant: ["ghost-muted"],
      class: "text-icon-default-muted",
    },
  ],
  defaultVariants: {
    variant: "primary",
    mode: "default",
    size: "default",
    shape: "rounded",
    inverted: false,
  },
});

export type ButtonVariantProps = VariantProps<typeof buttonVariants>;
export type ButtonStyles = ReturnType<typeof buttonVariants>;
