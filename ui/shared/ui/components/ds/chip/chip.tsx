"use client";

import type { ToggleProps } from "@radix-ui/react-toggle";
import * as Toggle from "@radix-ui/react-toggle";
import type {
  ToggleGroupMultipleProps,
  ToggleGroupSingleProps,
} from "@radix-ui/react-toggle-group";
import { type LucideIcon } from "lucide-react";
import type { ComponentProps, MouseEventHandler, ReactNode } from "react";
import { createElement, useContext, useMemo } from "react";

import { ToggleGroup, ToggleGroupItem } from "#components/ui/toggle-group.tsx";
import { cn } from "#utils/cn.ts";
import { createSlots } from "#utils/create-slots.tsx";

import {
  Tooltip as DsTooltip,
  TooltipContent,
  TooltipTrigger,
} from "../tooltip/tooltip";
import { ChipContext } from "./chip-context";
import { ChipGroupContext } from "./chip-group-context";
import { type ChipVariantProps, chipVariants } from "./chip-variants";

export type ChipProps = ComponentProps<typeof Toggle.Root> &
  ChipVariantProps &
  (
    | {
        value: string;
      }
    | {
        defaultPressed: Exclude<ToggleProps["defaultPressed"], undefined>;
      }
    | {
        pressed: Exclude<ToggleProps["pressed"], undefined>;
        onPressedChange: Exclude<ToggleProps["onPressedChange"], undefined>;
      }
  );

/**
 * A versatile Chip component that can be used as a standalone toggle or as part of a ChipGroup.
 *
 * @component
 * @param {Object} props - The component props
 * @param {React.ReactNode} props.children - The content of the chip
 * @param {string} [props.className] - Additional CSS classes to apply
 * @param {string} [props.variant='default'] - The visual style variant of the chip. Can be 'default', 'soft', 'ghost', 'brand', 'grey', or any color variant (e.g., 'red', 'orange', 'orangebright', 'yellow', 'lime', 'green', 'emerald', 'teal', 'cyan', 'sky', 'blue', 'bluedeep', 'purpledeep', 'purple', 'pinkdeep', 'pink', 'rose'). Color variants can also be prefixed with 'soft-' or 'ghost-' (e.g., 'soft-red', 'ghost-orange').
 * @param {'sm' | 'md' | 'lg'} [props.size='md'] - The size of the chip
 * @param {'rounded' | 'pill'} [props.shape='rounded'] - The shape of the chip
 * @param {string} [props.value] - Required when used within a ChipGroup to identify the chip
 * @param {boolean} [props.defaultPressed] - The default pressed state for standalone chips
 * @param {boolean} [props.pressed] - The controlled pressed state for standalone chips
 * @param {function} [props.onPressedChange] - Callback when the pressed state changes for standalone chips
 *
 * @example
 * // Standalone chip with default variant
 * <Chip defaultPressed={false}>
 *   Filter
 * </Chip>
 *
 * @example
 * // Standalone chip with soft variant and pill shape, and a tooltip
 * <Chip variant="soft" shape="pill" defaultPressed={false}>
 *   <ChipIcon icon={Filter} />
 *   Filter
 *   <ChipTooltip>I'm a tooltip!</ChipTooltip>
 * </Chip>
 *
 * @example
 * // Chip within a ChipGroup
 * <ChipGroup type="single">
 *   <Chip value="all">All</Chip>
 *   <Chip value="active">Active</Chip>
 *   <Chip value="completed">Completed</Chip>
 * </ChipGroup>
 *
 * @example
 * // Chip with orangebright color variant
 * <Chip variant="orangebright" defaultPressed={false}>
 *   Orange Chip
 * </Chip>
 */
const Chip = ({
  children,
  className,
  size,
  variant,
  shape,
  ...props
}: ChipProps) => {
  const chipGroupContext = useContext(ChipGroupContext);

  const slots = createSlots(children, {
    tooltip: ChipTooltip,
  });

  const contextValue = useMemo(() => {
    const styles =
      chipGroupContext?.styles ??
      chipVariants({ className, size, variant, shape });
    return { styles };
  }, [chipGroupContext?.styles, className, size, variant, shape]);

  /**
   * This is the case when the chip is used in a ChipGroup.
   * It must have a `value` prop to identify it.
   * Renders a radix-ui `ToggleGroupItem` component.
   */
  if (chipGroupContext) {
    if (!props.value || typeof props.value !== "string") {
      throw new Error("Chip must have a value when used in a ChipGroup");
    }

    return (
      <ChipContext.Provider value={contextValue}>
        {/* This code looks more complicated than needed, but there are incompatibilities between Toggle and Tootips */}
        {/* that require adaptations for each case (see the `asChild` prop not on the same element everytime) */}
        {/* @see https://github.com/radix-ui/primitives/issues/2153*/}
        {slots.tooltip ? (
          <DsTooltip disableHoverableContent>
            <ToggleGroupItem
              asChild
              {...props}
              value={props.value as string}
              className={contextValue.styles.base({ className })}
            >
              <TooltipTrigger>{slots.main}</TooltipTrigger>
            </ToggleGroupItem>
            {slots.tooltip}
          </DsTooltip>
        ) : (
          <ToggleGroupItem
            {...props}
            value={props.value as string}
            className={contextValue.styles.base({ className })}
          >
            {slots.main}
          </ToggleGroupItem>
        )}
      </ChipContext.Provider>
    );
  }

  /**
   * This is the case when the chip is used as a standalone component.
   * Renders a radix-ui `Toggle.Root` component.
   */
  return (
    <ChipContext.Provider value={contextValue}>
      {slots.tooltip ? (
        <DsTooltip disableHoverableContent>
          <Toggle.Root
            className={contextValue.styles.base({ className })}
            asChild
            {...props}
          >
            <TooltipTrigger>{slots.main}</TooltipTrigger>
          </Toggle.Root>
          {slots.tooltip}
        </DsTooltip>
      ) : (
        <Toggle.Root
          className={contextValue.styles.base({ className })}
          {...props}
        >
          {slots.main}
        </Toggle.Root>
      )}
    </ChipContext.Provider>
  );
};
Chip.displayName = "Chip";

type TooltipProps = { children: ReactNode } & ComponentProps<
  typeof TooltipContent
>;

/**
 * A tooltip component for Chip.
 * Must be used as a child of Chip.
 *
 * @component
 * @param {TooltipContentProps} props - The props of the underlying TooltipContent component
 * @param {React.ReactNode} props.children - The content of the tooltip
 *
 * @example
 * // A chip with a tooltip
 * <Chip>
 *   Chip with a tooltip
 *   <ChipTooltip>I'm a tooltip!</ChipTooltip>
 * </Chip>
 */
function ChipTooltip(props: TooltipProps) {
  return <TooltipContent {...props} />;
}
ChipTooltip.displayName = "ChipTooltip";

type IconProps = {
  icon: LucideIcon;
  className?: string;
  onClick?: MouseEventHandler<SVGElement>;
};

/**
 * Icon component for Chip.
 * Must be used as a child of Chip.
 *
 * @component
 * @param {Object} props - The component props
 * @param {LucideIcon} props.icon - The icon component to render
 * @param {string} [props.className] - Additional CSS classes to apply to the icon
 * @param {function} [props.onClick] - Callback when the icon is clicked
 *
 * @example
 * // Using with a Lucide icon
 * <Chip>
 *   <ChipIcon icon={Filter} />
 *   Filter
 * </Chip>
 */
function ChipIcon({ icon, className, onClick }: IconProps) {
  const context = useContext(ChipContext);
  if (!context) {
    throw new Error("ChipIcon must be used within a Chip");
  }

  return createElement(icon, {
    className: context.styles.icon({ className }),
    onClick,
  });
}
ChipIcon.displayName = "ChipIcon";

type ChipGroupProps = (ToggleGroupSingleProps | ToggleGroupMultipleProps) &
  ChipVariantProps & {
    segmented?: boolean;
  };

const segmentedChipGroupClasses =
  "[&>:not(:first-child)]:rounded-s-none [&>:not(:last-child)]:rounded-e-none [&>:not(:first-child)]:border-s-0";

/**
 * Chip Group component for creating a group of chips.
 *
 * @component
 * @param {Object} props - The component props
 * @param {React.ReactNode} props.children - The chips to include in the group
 * @param {string} [props.className] - Additional CSS classes to apply
 * @param {'default' | 'soft'} [props.variant='default'] - The visual style variant for all chips in the group
 * @param {'sm' | 'md' | 'lg'} [props.size='md'] - The size for all chips in the group
 * @param {'rounded' | 'pill'} [props.shape='rounded'] - The shape for all chips in the group
 * @param {boolean} [props.segmented=true] - Whether adjacent chips should visually collapse into a segmented control
 * @param {string} [props.value] - The selected value for single selection mode
 * @param {string[]} [props.value] - The selected values for multiple selection mode
 * @param {function} [props.onValueChange] - Callback when the selected value(s) change
 *
 * @example
 * // Single selection chip group
 * <ChipGroup type="single" value="all" onValueChange={(value) => console.log(value)}>
 *   <Chip value="all">All</Chip>
 *   <Chip value="active">Active</Chip>
 *   <Chip value="completed">Completed</Chip>
 * </ChipGroup>
 *
 * @example
 * // Multiple selection chip group with soft variant
 * <ChipGroup type="multiple" variant="soft" defaultValue={["all"]} onValueChange={(values) => console.log(values)}>
 *   <Chip value="all">All</Chip>
 *   <Chip value="active">Active</Chip>
 *   <Chip value="completed">Completed</Chip>
 * </ChipGroup>
 */
function ChipGroup({
  children,
  className,
  size,
  variant,
  shape,
  segmented = true,
  ...props
}: ChipGroupProps) {
  const contextValue = useMemo(() => {
    const styles = chipVariants({ className, size, variant, shape });
    return { styles };
  }, [className, size, variant, shape]);

  return (
    <ChipGroupContext.Provider value={contextValue}>
      <ToggleGroup
        className={cn(contextValue.styles.group({ className }), {
          [segmentedChipGroupClasses]: segmented,
        })}
        {...props}
      >
        {children}
      </ToggleGroup>
    </ChipGroupContext.Provider>
  );
}

ChipGroup.displayName = "ChipGroup";

export { Chip, ChipGroup, ChipIcon, ChipTooltip };
