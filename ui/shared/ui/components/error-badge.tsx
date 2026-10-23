/**
 * ErrorBadge component that displays an error message with a status indicator.
 * Typically used in forms.
 *
 * @component
 * @param {string} [props.message] - The error message to display
 *
 * @example
 * <ErrorBadge message="Invalid input" />
 */

import { cn } from "../utils/cn";
import type { BadgeProps } from "./ds/badge";
import { Badge, BadgeDot } from "./ds/badge";

interface ErrorBadgeProps extends BadgeProps {
  message?: string;
}

function ErrorBadge(props: ErrorBadgeProps) {
  const { message, className, ...otherProps } = props;
  return (
    <Badge
      variant="orange"
      size="sm"
      className={cn("py-1", className)}
      {...otherProps}
    >
      <BadgeDot />
      {message || ""}
    </Badge>
  );
}

ErrorBadge.displayName = "ErrorBadge";

export { ErrorBadge };
export type { ErrorBadgeProps };
