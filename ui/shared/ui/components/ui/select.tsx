"use client";

import * as SelectPrimitive from "@radix-ui/react-select";
import { Check, ChevronDown } from "lucide-react";
import * as React from "react";

import { cn } from "#utils/cn.ts";

import { FieldContext } from "../ds/form";

/**
 * @deprecated Use `Combobox` from `@mistral/ui/combobox` instead.
 */
const Select = SelectPrimitive.Root;

const SelectGroup = SelectPrimitive.Group;

const SelectValue = SelectPrimitive.Value;

export type SelectItemProps = React.ComponentPropsWithoutRef<
  typeof SelectPrimitive.Item
> & {
  noItemText?: boolean;
  checkPosition?: "start" | "end";
};

export type SelectTriggerProps = React.ComponentPropsWithoutRef<
  typeof SelectPrimitive.Trigger
> & {
  variant?: "default" | "soft";
};

const SelectTrigger = ({
  className,
  children,
  variant = "default",
  ref,
  ...props
}: SelectTriggerProps &
  Pick<React.ComponentProps<typeof SelectPrimitive.Trigger>, "ref">) => {
  const fieldContext = React.useContext(FieldContext);

  return (
    <SelectPrimitive.Trigger
      ref={ref}
      className={cn(
        "ring-offset-background data-placeholder:text-muted focus:ring-default flex h-10 w-full items-center justify-between gap-2 rounded-md px-3 py-2 text-sm focus:ring-2 focus:ring-offset-2 focus:outline-hidden disabled:cursor-not-allowed disabled:opacity-50 [&>span]:line-clamp-1",
        variant === "default" && [
          "border-default bg-default border",
          {
            "border-destructive": fieldContext?.isInvalid,
          },
        ],
        variant === "soft" && [
          "bg-state-soft text-default hover:bg-state-soft-hover active:bg-state-soft-press border-0",
        ],
        className,
      )}
      {...props}
    >
      {children}
      <SelectPrimitive.Icon asChild>
        <ChevronDown className="h-4 w-4 opacity-50" />
      </SelectPrimitive.Icon>
    </SelectPrimitive.Trigger>
  );
};
SelectTrigger.displayName = SelectPrimitive.Trigger.displayName;

const SelectContent = ({
  className,
  children,
  position = "popper",
  ref,
  ...props
}: React.ComponentProps<typeof SelectPrimitive.Content>) => (
  <SelectPrimitive.Portal>
    <SelectPrimitive.Content
      ref={ref}
      className={cn(
        "bg-default text-default data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95 data-[side=bottom]:slide-in-from-top-2 data-[side=left]:slide-in-from-right-2 data-[side=right]:slide-in-from-left-2 data-[side=top]:slide-in-from-bottom-2 relative z-50 max-h-96 min-w-[8rem] overflow-hidden rounded-md border shadow-md",
        position === "popper" &&
          "data-[side=bottom]:translate-y-1 data-[side=left]:-translate-x-1 data-[side=right]:translate-x-1 data-[side=top]:-translate-y-1",
        className,
      )}
      position={position}
      {...props}
    >
      <SelectPrimitive.Viewport
        className={cn(
          "p-1",
          position === "popper" &&
            "h-[var(--radix-select-trigger-height)] w-full min-w-[var(--radix-select-trigger-width)]",
        )}
      >
        {children}
      </SelectPrimitive.Viewport>
    </SelectPrimitive.Content>
  </SelectPrimitive.Portal>
);
SelectContent.displayName = SelectPrimitive.Content.displayName;

const SelectLabel = ({
  ref,
  className,
  ...props
}: React.ComponentProps<typeof SelectPrimitive.Label>) => (
  <SelectPrimitive.Label
    ref={ref}
    className={cn("py-1.5 pr-2 pl-8 text-sm font-semibold", className)}
    {...props}
  />
);
SelectLabel.displayName = SelectPrimitive.Label.displayName;

const SelectItem = ({
  className,
  children,
  noItemText,
  checkPosition = "start",
  ref,
  ...props
}: SelectItemProps &
  Pick<React.ComponentProps<typeof SelectPrimitive.Item>, "ref">) => (
  <SelectPrimitive.Item
    ref={ref}
    className={cn(
      "focus:bg-subtle focus:text-default hover:bg-subtle hover:text-default relative flex w-full cursor-pointer items-center rounded-sm py-1.5 pr-2 text-sm outline-hidden select-none data-disabled:pointer-events-none data-disabled:opacity-50",
      checkPosition === "start" ? "pl-8" : "pl-2",
      className,
    )}
    {...props}
  >
    {checkPosition === "start" && (
      <span className="absolute left-2 flex h-3.5 w-3.5 items-center justify-center">
        <SelectPrimitive.ItemIndicator>
          <Check className="h-4 w-4" />
        </SelectPrimitive.ItemIndicator>
      </span>
    )}

    {noItemText ? (
      children
    ) : (
      <SelectPrimitive.ItemText className="flex-1">
        {children}
      </SelectPrimitive.ItemText>
    )}

    {checkPosition === "end" && (
      <SelectPrimitive.ItemIndicator className="ms-auto ps-2">
        <Check className="h-4 w-4" />
      </SelectPrimitive.ItemIndicator>
    )}
  </SelectPrimitive.Item>
);

SelectItem.displayName = SelectPrimitive.Item.displayName;

const SelectSeparator = ({
  ref,
  className,
  ...props
}: React.ComponentProps<typeof SelectPrimitive.Separator>) => (
  <SelectPrimitive.Separator
    ref={ref}
    className={cn("bg-muted -mx-1 my-1 h-px", className)}
    {...props}
  />
);
SelectSeparator.displayName = SelectPrimitive.Separator.displayName;

const SelectItemText = SelectPrimitive.ItemText;

export {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectItemText,
  SelectLabel,
  SelectSeparator,
  SelectTrigger,
  SelectValue,
};
