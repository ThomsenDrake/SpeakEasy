"use client";

import { type DialogProps } from "@radix-ui/react-dialog";
import { Command as CommandPrimitive } from "cmdk";
import { Search } from "lucide-react";
import * as React from "react";

import { cn } from "#utils/cn.ts";

import type { InputVariantProps } from "../ds/input";
import { inputVariants } from "../ds/input";
import { Dialog, DialogContent } from "./dialog";

const Command = ({
  className,
  ref,
  ...props
}: React.ComponentProps<typeof CommandPrimitive>) => (
  <CommandPrimitive
    ref={ref}
    className={cn(
      "bg-card text-default flex h-full w-full flex-col overflow-hidden rounded-md",
      className,
    )}
    {...props}
  />
);
Command.displayName = CommandPrimitive.displayName;

type CommandDialogProps = DialogProps;

const CommandDialog = ({ children, ...props }: CommandDialogProps) => {
  return (
    <Dialog {...props}>
      <DialogContent className="overflow-hidden p-0 shadow-lg">
        <Command className="[&_[cmdk-group-heading]]:text-muted [&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:font-medium [&_[cmdk-group]]:px-2 [&_[cmdk-group]:not([hidden])_~[cmdk-group]]:pt-0 [&_[cmdk-input-wrapper]_svg]:h-5 [&_[cmdk-input-wrapper]_svg]:w-5 [&_[cmdk-input]]:h-12 [&_[cmdk-item]]:px-2 [&_[cmdk-item]]:py-3 [&_[cmdk-item]_svg]:h-5 [&_[cmdk-item]_svg]:w-5">
          {children}
        </Command>
      </DialogContent>
    </Dialog>
  );
};

const CommandInput = ({
  className,
  children,
  variant = "default",
  ref,
  ...props
}: React.ComponentProps<typeof CommandPrimitive.Input> & {
  children?: React.ReactNode;
  variant?: InputVariantProps["variant"];
}) => {
  const styles = inputVariants({ variant });
  return (
    <div
      className={cn(
        styles.container(),
        styles.wrapper(),
        cn(
          variant === "default" &&
            "border-default rounded-b-none border-0 border-b ring-0 ring-transparent has-[input:focus]:shadow-none [input:focus]:shadow-none",
          className,
        ),
      )}
    >
      <Search className={styles.leadIcon()} aria-hidden="true" />
      <CommandPrimitive.Input ref={ref} className={styles.input()} {...props} />
      {children}
    </div>
  );
};

CommandInput.displayName = CommandPrimitive.Input.displayName;

const CommandList = ({
  className,
  ref,
  ...props
}: React.ComponentProps<typeof CommandPrimitive.List>) => (
  <CommandPrimitive.List
    ref={ref}
    className={cn("max-h-[300px] overflow-x-hidden overflow-y-auto", className)}
    {...props}
  />
);

CommandList.displayName = CommandPrimitive.List.displayName;

const CommandEmpty = ({
  ref,
  ...props
}: React.ComponentProps<typeof CommandPrimitive.Empty>) => (
  <CommandPrimitive.Empty
    ref={ref}
    className="py-6 text-center text-sm"
    {...props}
  />
);

CommandEmpty.displayName = CommandPrimitive.Empty.displayName;

const CommandGroup = ({
  ref,
  className,
  ...props
}: React.ComponentProps<typeof CommandPrimitive.Group>) => (
  <CommandPrimitive.Group
    ref={ref}
    className={cn(
      "text-default [&_[cmdk-group-heading]]:text-muted overflow-hidden p-1 [&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:py-1.5 [&_[cmdk-group-heading]]:text-xs [&_[cmdk-group-heading]]:font-medium",
      className,
    )}
    {...props}
  />
);

CommandGroup.displayName = CommandPrimitive.Group.displayName;

const CommandSeparator = ({
  ref,
  className,
  ...props
}: React.ComponentProps<typeof CommandPrimitive.Separator>) => (
  <CommandPrimitive.Separator
    ref={ref}
    className={cn("border-default -mx-1 border", className)}
    {...props}
  />
);
CommandSeparator.displayName = CommandPrimitive.Separator.displayName;

const CommandItem = ({
  ref,
  className,
  ...props
}: React.ComponentProps<typeof CommandPrimitive.Item>) => (
  <CommandPrimitive.Item
    ref={ref}
    className={cn(
      "aria-selected:bg-subtle aria-selected:text-default relative flex cursor-default items-center rounded-sm px-2 py-1.5 text-sm outline-hidden select-none data-[disabled=true]:pointer-events-none data-[disabled=true]:opacity-50",
      className,
    )}
    {...props}
  />
);

CommandItem.displayName = CommandPrimitive.Item.displayName;

const CommandLoading = ({
  className,
  ...props
}: React.ComponentProps<typeof CommandPrimitive.Loading>) => (
  <CommandPrimitive.Loading
    className={cn(
      "text-muted flex items-center justify-center gap-2 py-6 text-sm",
      className,
    )}
    {...props}
  />
);
CommandLoading.displayName = CommandPrimitive.Loading.displayName;

const CommandShortcut = ({
  className,
  ...props
}: React.HTMLAttributes<HTMLSpanElement>) => {
  return (
    <span
      className={cn("text-muted ml-auto text-xs tracking-widest", className)}
      {...props}
    />
  );
};
CommandShortcut.displayName = "CommandShortcut";

export {
  Command,
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandLoading,
  CommandSeparator,
  CommandShortcut,
};
