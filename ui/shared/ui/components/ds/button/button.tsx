"use client";

import { AnimatePresence, motion } from "framer-motion";
import type { LucideIcon } from "lucide-react";
import { LoaderCircle } from "lucide-react";
import type {
  ComponentProps,
  ComponentPropsWithRef,
  ReactElement,
  ReactNode,
} from "react";
import { createElement, isValidElement, useContext, useMemo } from "react";

import { cn } from "../../../utils/cn";
import { createSlots } from "../../../utils/create-slots";
import type { TooltipProps } from "../tooltip/tooltip";
import {
  Tooltip as DsTooltip,
  TooltipContent,
  TooltipTrigger,
} from "../tooltip/tooltip";
import { ButtonContext } from "./button-context";
import { ButtonGroupContext } from "./button-group-context";
import type { ButtonVariantProps } from "./button-variants";
import { buttonVariants } from "./button-variants";

export type ButtonProps = Omit<ComponentPropsWithRef<"button">, "disabled"> & {
  tooltipRoot?: TooltipProps;
} & Omit<ButtonVariantProps, "mode"> & {
    isLoading?: boolean;
    isDisabled?: boolean;
  } & (
    | {
        mode: "icon-only";
        icon: LucideIcon | ReactElement;
      }
    | {
        mode?: Exclude<ButtonVariantProps["mode"], "icon-only">;
        icon?: undefined;
      }
  );

/**
 * A versatile Button component with multiple variants, sizes, and states.
 *
 * @component
 * @see https://www.figma.com/design/FZxG4186pzrdTECGoeqYbH/%F0%9F%8E%A8-Sort-1.2---Mistral?node-id=28-22033&m=dev
 * @param {Object} props - The component props
 * @param {boolean} [props.isLoading] - Whether the button is in a loading state
 * @param {boolean} [props.isDisabled] - Whether the button is disabled
 * @param {React.ReactNode} props.children - The content of the button
 * @param {'primary' | 'secondary' | 'dashed' | 'brand' | 'soft' | 'ghost' | 'destructive'} [props.variant='primary'] - The visual style variant of the button
 * @param {'rounded' | 'pill'} [props.shape='rounded'] - The shape of the button
 * @param {'2xs' | 'xs' | 'sm' | 'default' | 'lg'} [props.size='default'] - The size of the button
 * @param {'default' | 'icon-only'} [props.mode='default'] - The mode of the button
 * @param {string} [props.className] - Additional CSS classes to apply
 * @param {LucideIcon | ReactNode} [props.icon] - Icon to display (required when mode is 'icon-only')
 *
 * @example
 * // Basic button with text
 * <Button variant="primary" size="default">
 *   Click me
 * </Button>
 *
 * @example
 * // Button with leading and trailing icons, and a tooltip
 * <Button variant="secondary">
 *   <ButtonLeadIcon icon={Settings} />
 *   Settings
 *   <ButtonTailIcon icon={ArrowRight} />
 *   <ButtonTooltip>I'm a tooltip!</ButtonTooltip>
 * </Button>
 *
 * @example
 * // Icon-only button
 * <Button variant="soft" mode="icon-only" icon={Home} />
 * @example
 * // Icon-only button using a custom ReactNode
 * <Button variant="soft" mode="icon-only" icon={<div className="size-3">@</div>} />
 */
const Button = ({
  isLoading,
  isDisabled,
  children,
  variant,
  inverted,
  shape,
  size,
  mode,
  className,
  icon,
  tooltipRoot,
  ...props
}: ButtonProps) => {
  const buttonGroupContext = useContext(ButtonGroupContext);

  const slots = createSlots(children, {
    tailIcon: ButtonTailIcon,
    leadIcon: ButtonLeadIcon,
    tooltip: ButtonTooltip,
  });

  const contextValue = useMemo(
    () => ({
      styles:
        buttonGroupContext?.styles ??
        buttonVariants({ shape, size, mode, variant, inverted }),
    }),
    [shape, size, mode, variant, inverted, buttonGroupContext],
  );

  const buttonContent = (
    <button
      aria-disabled={isDisabled}
      aria-busy={isLoading}
      disabled={isDisabled || isLoading}
      type="button"
      className={cn(contextValue.styles.base({ className }))}
      {...props}
    >
      <AnimatePresence>
        {isLoading && (
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1, transition: { delay: 0.15 } }}
            className={contextValue.styles.spinner()}
            aria-disabled={isDisabled}
          >
            <LoaderCircle className="size-4 animate-spin" />
          </motion.div>
        )}
      </AnimatePresence>

      {mode === "icon-only" ? (
        isValidElement(icon) ? (
          icon
        ) : (
          createElement(icon, { className: contextValue.styles.onlyIcon() })
        )
      ) : (
        <>
          {slots.leadIcon}
          {slots.main}
          {slots.tailIcon}
        </>
      )}
    </button>
  );

  return (
    <ButtonContext.Provider value={contextValue}>
      {slots.tooltip ? (
        <DsTooltip
          delayDuration={(slots.tooltip.props as TooltipProps).delayDuration}
          {...tooltipRoot}
        >
          <TooltipTrigger asChild>{buttonContent}</TooltipTrigger>
          {slots.tooltip}
        </DsTooltip>
      ) : (
        buttonContent
      )}
    </ButtonContext.Provider>
  );
};
Button.displayName = "Button";

type TooltipContentProps = {
  children: ReactNode;
  delayDuration?: number;
} & ComponentProps<typeof TooltipContent>;

/**
 * A tooltip component for Button.
 * Must be used as a child of Button.
 *
 * @component
 * @param {TooltipContentProps} props - The props of the underlying TooltipContent component
 * @param {React.ReactNode} props.children - The content of the tooltip
 *
 * @example
 * // A button with a tooltip
 * <Button>
 *   Click me
 *   <ButtonTooltip
 *     align="end"
 *     alignOffset={12}
 *     delayDuration={250}
 *   >
 *     I'm a tooltip!
 *   </ButtonTooltip>
 * </Button>
 */
function ButtonTooltip({ delayDuration: _, ...props }: TooltipContentProps) {
  return <TooltipContent {...props} />;
}
ButtonTooltip.displayName = "ButtonTooltip";

type IconProps =
  | { icon: LucideIcon; className?: string }
  | { children: ReactNode };

/**
 * Lead icon component for Button.
 * Must be used as a child of Button.
 *
 * @component
 * @param {LucideIcon} [props.icon] - The icon component to render
 * @param {string} [props.className] - Additional CSS classes to apply to the icon
 * @param {React.ReactNode} [props.children] - Custom content to render instead of an icon
 *
 * @example
 * // Using with a Lucide icon
 * <ButtonLeadIcon icon={Settings} />
 *
 * @example
 * // Using with custom content
 * <ButtonLeadIcon>
 *   <Loader2 className="size-4 animate-spin" />
 * </ButtonLeadIcon>
 */
function ButtonLeadIcon(props: IconProps) {
  const context = useContext(ButtonContext);

  if (!context) {
    throw new Error("ButtonLeadIcon must be used within a Button");
  }

  return "icon" in props
    ? createElement(props.icon, {
        className: context.styles.asideIcon({ className: props.className }),
      })
    : props.children;
}
ButtonLeadIcon.displayName = "ButtonLeadIcon";

/**
 * Tail icon component for Button.
 * Must be used as a child of Button.
 *
 * @component
 * @param {LucideIcon} [props.icon] - The icon component to render
 * @param {string} [props.className] - Additional CSS classes to apply to the icon
 * @param {React.ReactNode} [props.children] - Custom content to render instead of an icon
 *
 * @example
 * // Using with a Lucide icon
 * <ButtonTailIcon icon={ArrowRight} />
 *
 * @example
 * // Using with custom content
 * <ButtonTailIcon>
 *   <Loader2 className="size-4 animate-spin" />
 * </ButtonTailIcon>
 */
function ButtonTailIcon(props: IconProps) {
  const context = useContext(ButtonContext);

  if (!context) {
    throw new Error("ButtonTailIcon must be used within a Button");
  }

  return "icon" in props
    ? createElement(props.icon, {
        className: context.styles.asideIcon({ className: props.className }),
      })
    : props.children;
}
ButtonTailIcon.displayName = "ButtonTailIcon";

export interface ButtonGroupProps extends ButtonVariantProps {
  children: React.ReactNode[];
  className?: string;
}

/**
 * Button Group component for creating a connected group of buttons.
 *
 * @component
 * @param {React.ReactNode[]} props.children - The buttons to include in the group
 * @param {string} [props.className] - Additional CSS classes to apply
 * @param {'primary' | 'secondary' | 'dashed' | 'brand' | 'soft' | 'ghost' | 'destructive'} [props.variant='primary'] - The visual style variant for all buttons in the group
 * @param {'rounded' | 'pill'} [props.shape='rounded'] - The shape for all buttons in the group
 * @param {'2xs' | 'xs' | 'sm' | 'default' | 'lg'} [props.size='default'] - The size for all buttons in the group
 * @param {'default' | 'icon-only'} [props.mode='default'] - The mode for all buttons in the group
 *
 * @example
 * // Basic button group
 * <ButtonGroup variant="secondary" size="sm">
 *   <Button>Option 1</Button>
 *   <Button>Option 2</Button>
 *   <Button>Option 3</Button>
 * </ButtonGroup>
 *
 * @example
 * // Icon-only button group
 * <ButtonGroup variant="soft" shape="pill">
 *   <Button mode="icon-only" icon={List} />
 *   <Button mode="icon-only" icon={Grid} />
 *   <Button mode="icon-only" icon={LayoutGrid} />
 * </ButtonGroup>
 */
function ButtonGroup({
  children,
  className,
  size,
  variant,
  mode,
  shape,
  inverted,
}: ButtonGroupProps) {
  const contextValue = useMemo(
    () => ({
      styles: buttonVariants({
        className,
        size,
        variant,
        mode,
        shape,
        inverted,
      }),
    }),
    [className, size, variant, mode, shape, inverted],
  );

  return (
    <ButtonGroupContext.Provider value={contextValue}>
      <div className={contextValue.styles.group()}>{children}</div>
    </ButtonGroupContext.Provider>
  );
}
ButtonGroup.displayName = "ButtonGroup";

/**
 * This component wraps its children in an empty button group context provider.
 * It can be used to reset the context and prevent the buttons in the underlying
 * components from being styled by a parent button group.
 *
 * @component
 * @param {React.ReactNode} props.children - The child tree to reset the context for.
 *
 * @example
 * // A dialog with a button group
 * <ButtonGroupEmptyProvider>
 *   <Dialog.Content>...</Dialog.Content>
 * </ButtonGroupEmptyProvider>
 */
function ButtonGroupEmptyProvider({ children }: { children: ReactNode }) {
  return (
    <ButtonGroupContext.Provider value={null}>
      {children}
    </ButtonGroupContext.Provider>
  );
}

export {
  Button,
  ButtonGroup,
  ButtonGroupEmptyProvider,
  ButtonLeadIcon,
  ButtonTailIcon,
  ButtonTooltip,
};
