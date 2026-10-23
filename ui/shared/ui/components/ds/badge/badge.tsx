"use client";

import { type LucideIcon } from "lucide-react";
import { createElement, useContext, useMemo } from "react";

import { BadgeContext } from "./badge-context";
import { type BadgeVariantProps, badgeVariants } from "./badge-variants";

export type BadgeProps = React.HTMLAttributes<HTMLDivElement> &
  BadgeVariantProps;

/**
 * Badge component for highlighting status, count, or categorizing items with visual indicators.
 *
 * @component
 * @see https://www.figma.com/design/FZxG4186pzrdTECGoeqYbH/%F0%9F%8E%A8-Sort-1.2---Mistral?node-id=3994-37188&m=dev
 * @param {string} [props.className] - Additional CSS classes
 * @param {"neutral" | "red" | "orange" | "orangebright" | "yellow" | "lime" | "green" |
 *   "emerald" | "teal" | "cyan" | "sky" | "blue" | "bluedeep" | "purpledeep" | "purple" |
 *   "pinkdeep" | "pink" | "rose"} [props.variant='neutral'] - Color scheme of the badge
 * @param {boolean} [props.bordered=false] - Whether the badge has a border
 * @param {'xs' | 'sm' | 'lg'} [props.size='lg'] - Size of the badge
 * @param {'rounded' | 'pill'} [props.shape='rounded'] - Shape of the badge
 * @param {React.ReactNode} props.children - Content to display inside the badge
 *
 * @example
 * // Basic badge with text
 * <Badge>Default</Badge>
 *
 * @example
 * // Colored badge with status dot and custom styling
 * <Badge
 *   variant="green"
 *   bordered={true}
 *   shape="pill"
 *   size="sm"
 * >
 *   <BadgeDot />
 *   Online
 * </Badge>
 */
const Badge = ({
  children,
  className,
  variant,
  bordered,
  size,
  ...props
}: BadgeProps) => {
  const contextValue = useMemo(
    () => ({
      styles: badgeVariants({ className, variant, bordered, size }),
    }),
    [className, variant, bordered, size],
  );

  return (
    <BadgeContext.Provider value={contextValue}>
      <span
        role="status"
        className={contextValue.styles.base({ className })}
        {...props}
      >
        {children}
      </span>
    </BadgeContext.Provider>
  );
};
Badge.displayName = "Badge";

type IconProps = { icon: LucideIcon; className?: string };

/**
 * Icon component that displays a Lucide icon within the badge.
 * Must be used within a Badge.
 *
 * @component
 * @param {LucideIcon} props.icon - The Lucide icon component to render
 * @param {string} [props.className] - Additional CSS classes
 *
 * @example
 * <Badge variant="blue">
 *   <BadgeIcon icon={Info} />
 *   Information
 * </Badge>
 */
function BadgeIcon({ icon, className }: IconProps) {
  const context = useContext(BadgeContext);

  if (!context) {
    throw new Error("BadgeIcon must be used within a Badge");
  }

  return createElement(icon, {
    className: context.styles.icon({ className }),
  });
}
BadgeIcon.displayName = "BadgeIcon";

type DotProps = { className?: string };

/**
 * Dot component that displays a colored status indicator within the badge.
 * Must be used within a Badge.
 *
 * @component
 * @param {string} [props.className] - Additional CSS classes
 *
 * @example
 * <Badge variant="green">
 *   <BadgeDot />
 *   Active
 * </Badge>
 */
function BadgeDot({ className }: DotProps) {
  const context = useContext(BadgeContext);

  if (!context) {
    throw new Error("BadgeDot must be used within a Badge");
  }

  return (
    <svg
      className={context.styles.dot({ className })}
      viewBox="0 0 100 100"
      xmlns="http://www.w3.org/2000/svg"
    >
      <circle cx="50" cy="50" r="45" fill="currentColor" stroke="none" />
    </svg>
  );
}
BadgeDot.displayName = "BadgeDot";

export { Badge, BadgeDot, BadgeIcon };
