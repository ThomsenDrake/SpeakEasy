import type { VariantProps } from "class-variance-authority";
import { tv } from "tailwind-variants";

export const toastVariant = tv({
  slots: {
    base: "relative flex flex-row items-start bg-card gap-3 rounded-[var(--radius-card-md)] border-[0.5px] border-default p-4 w-96 w-full min-w-88 max-w-96 ",
    icon: "size-5",
    content: "flex flex-col flex-wrap gap-3 items-start w-full",
    text_container: "flex flex-col gap-1 min-w-0",
    buttonContainer: "flex flex-row gap-2",
    iconContainer: "size-5 text-sm",
    closeButton: "absolute right-1 top-1",
    closeContainer: "w-2",
  },
  variants: {
    isOneLiner: {
      true: {
        base: "items-center",
        content: "flex-row items-center justify-between",
      },
    },
    isDismissible: {
      true: {
        content: "flex-col items-start",
      },
    },
    variant: {
      success: {
        icon: "text-icon-success",
      },
      error: {
        icon: "text-icon-destructive",
      },
      warning: {
        icon: "text-icon-warning",
      },
      info: {},
      loading: {
        icon: "animate-spin",
      },
    },
  },
  defaultVariants: {
    variant: "success",
  },
});

export type ToastVariantProps = VariantProps<typeof toastVariant>;
export type ToastStyles = ReturnType<typeof toastVariant>;
