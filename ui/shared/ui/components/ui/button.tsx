import { Slot } from "@radix-ui/react-slot";
import { cva, type VariantProps } from "class-variance-authority";
import { LoaderCircle } from "lucide-react";
import * as React from "react";

import { cn } from "#utils/cn.ts";

import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "../ds/tooltip";

const buttonVariants = cva(
  "disabled:pointer-auto inline-flex items-center justify-center gap-2 rounded-md text-sm leading-5.5 font-normal whitespace-nowrap transition-colors focus-visible:outline-hidden disabled:opacity-50 [&_svg]:size-4 [&_svg]:shrink-0",
  {
    variants: {
      variant: {
        default:
          "border-brand text-brand-600 hover:bg-brand-100/80 bg-brand-100/60 dark:bg-brand-900/60 dark:hover:bg-brand-900/80 dark:text-brand-300 border",
        "default-c":
          "border-brand/20 bg-state-brand/10 text-brand-600 hover:bg-state-brand/20 border",
        destructive:
          "bg-state-destructive text-white-default hover:bg-state-destructive/90",
        success:
          "bg-(--green-500)/50 text-(--green-500) hover:bg-(--green-500)/90",
        outline: "border-default bg-default text-default hover:bg-muted border",
        "outline-c":
          "border-default bg-default hover:bg-subtle hover:text-default border",
        secondary:
          "border-default bg-muted text-muted hover:border-default hover:text-default border hover:bg-(--secondary-200)",
        "secondary-c": "bg-subtle text-secondary-foreground hover:bg-subtle/80",
        ghost: "text-subtle disabled:text-hint hover:bg-subtle",
        "ghost-c": "hover:bg-subtle hover:text-default",
        link: "text-brand-600 underline-offset-4 hover:underline",
        dark: "bg-inverted text-inverted-default",
      },
      size: {
        default: "h-10 px-4 py-2",
        sm: "h-9 px-3",
        lg: "h-11 px-8",
        icon: "h-10 w-10",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  },
);

export interface ButtonProps
  extends
    React.ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof buttonVariants> {
  asChild?: boolean;
  tooltip?: string;
  isLoading?: boolean;
}

/**
 * @deprecated ⚠️ Use `Button` from `@mistral/ui/button` instead ⚠️
 */
const Button = ({
  className,
  variant,
  size,
  asChild = false,
  tooltip,
  disabled,
  isLoading,
  children,
  ref,
  ...props
}: ButtonProps & { ref?: React.Ref<HTMLButtonElement> }) => {
  const Comp = asChild ? Slot : "button";
  const buttonContent = (
    <>
      {isLoading && <LoaderCircle className="size-4 animate-spin" />}
      {children}
    </>
  );

  const button = (
    <Comp
      className={cn(buttonVariants({ variant, size, className }))}
      ref={ref}
      disabled={disabled || isLoading}
      type="button"
      {...props}
    >
      {asChild ? children : buttonContent}
    </Comp>
  );

  if (tooltip) {
    return (
      <TooltipProvider delayDuration={0}>
        <Tooltip>
          <TooltipTrigger asChild>{button}</TooltipTrigger>
          <TooltipContent>{tooltip}</TooltipContent>
        </Tooltip>
      </TooltipProvider>
    );
  }

  return button;
};
Button.displayName = "Button";

export { Button, buttonVariants };
