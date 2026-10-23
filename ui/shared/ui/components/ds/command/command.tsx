"use client";
import { Command as CommandPrimitive } from "cmdk";
import { Loader2, type LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import { createElement, useContext, useMemo } from "react";

import { Dialog, DialogContent, DialogTitle } from "#components/ui/dialog.tsx";
import { VisuallyHidden } from "#index.ts";

import { Input, InputContainer } from "../input";
import { CommandContext } from "./command-context";
import type { CommandVariantProps } from "./command-variants";
import { commandVariants } from "./command-variants";

interface CommandProps
  extends React.ComponentProps<typeof CommandPrimitive>, CommandVariantProps {}

function Command({ className, size, ...props }: CommandProps) {
  const contextValue = useMemo(() => {
    const styles = commandVariants({ size });
    return { styles, variants: { size } };
  }, [size]);

  return (
    <CommandContext.Provider value={contextValue}>
      <CommandPrimitive
        className={contextValue.styles.command({ className })}
        {...props}
      />
    </CommandContext.Provider>
  );
}

function CommandInput({
  className,
  ...props
}: React.ComponentProps<typeof CommandPrimitive.Input>) {
  const context = useContext(CommandContext);

  if (!context) {
    throw new Error("CommandInput must be used within a Command");
  }

  return (
    <InputContainer className={context.styles.inputContainer({ className })}>
      <CommandPrimitive.Input asChild {...props}>
        <Input size={context.variants.size === "lg" ? "lg" : "sm"} />
      </CommandPrimitive.Input>
    </InputContainer>
  );
}

function CommandList({
  className,
  ...props
}: React.ComponentProps<typeof CommandPrimitive.List>) {
  const context = useContext(CommandContext);

  if (!context) {
    throw new Error("CommandList must be used within a Command");
  }

  return (
    <CommandPrimitive.List
      className={context.styles.list({ className })}
      {...props}
    />
  );
}

function CommandEmpty({
  className,
  ...props
}: React.ComponentProps<typeof CommandPrimitive.Empty>) {
  const context = useContext(CommandContext);

  if (!context) {
    throw new Error("CommandEmpty must be used within a Command");
  }

  return (
    <CommandPrimitive.Empty
      className={context.styles.empty({ className })}
      {...props}
    />
  );
}

function CommandGroup({
  className,
  ...props
}: React.ComponentProps<typeof CommandPrimitive.Group>) {
  const context = useContext(CommandContext);

  if (!context) {
    throw new Error("CommandGroup must be used within a Command");
  }

  return (
    <CommandPrimitive.Group
      className={context.styles.group({ className })}
      {...props}
    />
  );
}

function CommandItem({
  className,
  ...props
}: React.ComponentProps<typeof CommandPrimitive.Item>) {
  const context = useContext(CommandContext);

  if (!context) {
    throw new Error("CommandItem must be used within a Command");
  }

  return (
    <CommandPrimitive.Item
      className={context.styles.item({ className })}
      {...props}
    />
  );
}

type IconProps =
  | { icon: LucideIcon; className?: string }
  | { children: ReactNode };

function CommandItemLeadIcon(props: IconProps) {
  const context = useContext(CommandContext);

  if (!context) {
    throw new Error("CommandItemLeadIcon must be used within a CommandItem");
  }

  return "icon" in props
    ? createElement(props.icon, {
        className: context.styles.itemAsideIcon({ className: props.className }),
      })
    : props.children;
}
CommandItemLeadIcon.displayName = "CommandItemLeadIcon";

function CommandItemTailIcon(props: IconProps) {
  const context = useContext(CommandContext);

  if (!context) {
    throw new Error("CommandItemTailIcon must be used within a CommandItem");
  }

  return "icon" in props
    ? createElement(props.icon, {
        className: context.styles.itemAsideIcon({ className: props.className }),
      })
    : props.children;
}
CommandItemTailIcon.displayName = "CommandItemTailIcon";

function CommandSeparator({
  className,
  ...props
}: React.ComponentProps<typeof CommandPrimitive.Separator>) {
  const context = useContext(CommandContext);

  if (!context) {
    throw new Error("CommandSeparator must be used within a Command");
  }

  return (
    <CommandPrimitive.Separator
      className={context.styles.separator({ className })}
      {...props}
    />
  );
}

function CommandDialog({
  children,
  className,
  ...props
}: React.ComponentProps<typeof Dialog> & {
  children: React.ReactNode;
  className?: string;
}) {
  const styles = commandVariants();

  return (
    <Dialog {...props}>
      <DialogContent
        hideX
        className={styles.dialogCommandContent({ className })}
        onEscapeKeyDown={(e) => {
          e.preventDefault();
        }}
      >
        <VisuallyHidden>
          <DialogTitle></DialogTitle>
        </VisuallyHidden>
        {children}
      </DialogContent>
    </Dialog>
  );
}

function CommandLoading({
  className,
  children,
  ...props
}: React.ComponentProps<typeof CommandPrimitive.Loading>) {
  const context = useContext(CommandContext);

  if (!context) {
    throw new Error("CommandLoading must be used within a Command");
  }

  return (
    <CommandPrimitive.Loading {...props}>
      <div className={context.styles.loading({ className })}>
        <CommandItemLeadIcon>
          <Loader2 className="size-[1em] animate-spin" />
        </CommandItemLeadIcon>
        <span>{children}</span>
      </div>
    </CommandPrimitive.Loading>
  );
}

export {
  Command,
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandItemLeadIcon,
  CommandItemTailIcon,
  CommandList,
  CommandLoading,
  CommandSeparator,
};
