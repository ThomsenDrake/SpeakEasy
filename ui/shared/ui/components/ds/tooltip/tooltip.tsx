"use client";

import * as TooltipPrimitive from "@radix-ui/react-tooltip";
import { type ComponentProps, useContext, useMemo } from "react";

import { createSlots } from "#utils/create-slots.tsx";

import { Badge, type BadgeProps } from "../badge/badge";
import { TooltipContext } from "./tooltip-context";
import { type TooltipVariantProps, tooltipVariants } from "./tooltip-variants";

export type TooltipProps = TooltipPrimitive.TooltipProps & TooltipVariantProps;

/**
 * Tooltip component for displaying additional information when users hover over or focus on an element.
 *
 * @component
 * @see https://www.figma.com/design/FZxG4186pzrdTECGoeqYbH/%F0%9F%8E%A8-Sort-1.2---Mistral?node-id=3844-16389&m=dev
 * @param {ReactNode} props.children - Content to be wrapped by the tooltip (usually TooltipTrigger and TooltipContent)
 *
 * @example
 * // Basic tooltip
 * <Tooltip>
 *   <TooltipTrigger>Hover me</TooltipTrigger>
 *   <TooltipContent>Tooltip content</TooltipContent>
 * </Tooltip>
 */
const Tooltip = ({ children, ...props }: TooltipProps) => {
  const contextValue = useMemo(() => {
    const styles = tooltipVariants();
    return { styles };
  }, []);

  return (
    <TooltipContext.Provider value={contextValue}>
      <TooltipPrimitive.Root {...props}>{children}</TooltipPrimitive.Root>
    </TooltipContext.Provider>
  );
};
Tooltip.displayName = "Tooltip";

type ContentProps = ComponentProps<typeof TooltipPrimitive.Content>;

/**
 * Content component that displays the tooltip information.
 * Must be used within a Tooltip.
 *
 * @component
 * @param {string} [props.className] - Additional CSS classes
 * @param {number} [props.sideOffset=4] - Offset from the trigger element
 *
 * @example
 * // with some style overrides
 * <Tooltip>
 *   <TooltipTrigger>Hover me</TooltipTrigger>
 *   <TooltipContent className="bg-muted text-muted">
 *     This is the tooltip content
 *   </TooltipContent>
 * </Tooltip>
 */
function TooltipContent({
  className,
  sideOffset = 4,
  children,
  ...props
}: ContentProps) {
  const tooltipContext = useContext(TooltipContext);

  if (!tooltipContext) {
    throw new Error("TooltipContent must be used within a Tooltip");
  }

  const slots = createSlots(children, { badge: TooltipBadge });

  return (
    <TooltipPrimitive.Portal>
      <TooltipPrimitive.Content
        sideOffset={sideOffset}
        className={tooltipContext.styles.content({ className })}
        {...props}
      >
        {slots.main}
        {slots.badge}
      </TooltipPrimitive.Content>
    </TooltipPrimitive.Portal>
  );
}
TooltipContent.displayName = "TooltipContent";

/**
 * Badge component that displays an indicator for the Tooltip.
 * Must be used within TooltipContent.
 *
 * @component
 * @param {string} [props.className] - Additional CSS classes
 *
 * @example
 * // A badge to show a keyboard shortcut
 * <Tooltip>
 *   <TooltipTrigger>Hover me</TooltipTrigger>
 *   <TooltipContent>
 *     <TooltipBadge>
 *       Cmd+S
 *     </TooltipBadge>
 *     Save changes
 *   </TooltipContent>
 * </Tooltip>
 */
export function TooltipBadge({
  className,
  ...props
}: Omit<BadgeProps, "size" | "variant" | "bordered">) {
  const tooltipContext = useContext(TooltipContext);

  if (!tooltipContext) {
    throw new Error("TooltipBadge must be used within a Tooltip");
  }

  return (
    <Badge className={tooltipContext.styles.badge({ className })} {...props} />
  );
}
TooltipBadge.displayName = "TooltipBadge";

/**
 * Provider component for tooltips that allows configuring global tooltip behavior.
 * Wraps the application or a section where tooltips are used.
 */
const TooltipProvider = TooltipPrimitive.Provider;

/**
 * Trigger component that users interact with to show the tooltip.
 * Must be used within a Tooltip.
 * Render a button, or takes the `asChild` prop to spread its props and behaviour on its single child element.
 *
 * @component
 * @param {boolean} [props.asChild] - Spread props and behaviour on the single child element
 *
 *
 * @example
 * // Basic tooltip trigger
 * <Tooltip>
 *   <TooltipTrigger>
 *     <Button>Hover for info</Button>
 *   </TooltipTrigger>
 *   <TooltipContent>More information</TooltipContent>
 * </Tooltip>
 *
 * @example
 * // Tooltip trigger with custom element
 * <Tooltip>
 *   <TooltipTrigger asChild>
 *     <Button onClick={() => {}}>Hover for info</Button>
 *   </TooltipTrigger>
 *   <TooltipContent>More information</TooltipContent>
 * </Tooltip>
 */
const TooltipTrigger = TooltipPrimitive.Trigger;

export { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger };
