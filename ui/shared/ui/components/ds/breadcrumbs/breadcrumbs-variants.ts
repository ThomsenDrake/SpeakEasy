import type { VariantProps } from "class-variance-authority";
import { tv } from "tailwind-variants";

export const breadcrumbsVariants = tv({
  slots: {
    root: "flex items-center gap-3 py-1",
    item: "flex items-center text-sm gap-3 data-[last-breadcrumbs-item]:text-muted font-medium",
    separator: "text-hint",
  },
  variants: {},
  defaultVariants: {},
});

export type BreadcrumbsVariantProps = VariantProps<typeof breadcrumbsVariants>;
export type BreadcrumbsStyles = ReturnType<typeof breadcrumbsVariants>;
