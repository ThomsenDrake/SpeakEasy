"use client";

import {
  Button as ButtonUi,
  buttonVariants,
} from "#components/ds/button/index.ts";
import {
  Tooltip,
  TooltipContent,
  type TooltipProps,
  TooltipTrigger,
} from "#components/ds/tooltip/tooltip.tsx";
import { cn } from "#utils/cn.ts";

export const barContainerStyle = cn(
  "top-0 right-0 left-0 z-20 flex w-full flex-row items-center justify-between transition-shadow",
);

export const Section = ({
  children,
  className,
}: {
  children: React.ReactNode;
  className?: string;
}) => {
  return (
    <div className={cn("flex flex-row items-center p-3", className)}>
      {children}
    </div>
  );
};

export const Fill = () => {
  return <div className="flex-1 gap-1 py-3" />;
};

export function Button({
  disabled,
  className,
  onClick,
  tooltipText,
  children,
  tooltipProps,
}: {
  disabled?: boolean;
  className?: string;
  onClick?: () => void;
  tooltipText?: string;
  children: React.ReactNode;
  tooltipProps?: TooltipProps;
}) {
  return (
    <Tooltip {...tooltipProps}>
      <TooltipTrigger asChild disabled={disabled}>
        <ButtonUi
          isDisabled={disabled}
          aria-label={tooltipText}
          onClick={onClick}
          variant="ghost"
          size="sm"
          className={cn(
            buttonVariants({ variant: "ghost", size: "sm" }),
            "flex h-auto w-auto py-2",
            disabled ? "pointer-events-none opacity-50" : "",
            className,
          )}
        >
          {children}
        </ButtonUi>
      </TooltipTrigger>
      {tooltipText && (
        <TooltipContent side="bottom">
          <p>{tooltipText}</p>
        </TooltipContent>
      )}
    </Tooltip>
  );
}
