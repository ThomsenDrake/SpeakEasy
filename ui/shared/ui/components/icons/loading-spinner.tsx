import type { ComponentPropsWithoutRef } from "react";

import { cn } from "../../utils/cn";

interface LoadingSpinnerProps extends ComponentPropsWithoutRef<"svg"> {
  className?: string;
  width?: string | number;
  height?: string | number;
  strokeColor?: string;
}

export const LoadingSpinner: React.FC<LoadingSpinnerProps> = ({
  className,
  width = 24,
  height = 24,
  strokeColor = "currentColor",
  ...props
}) => {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      width={width}
      height={height}
      viewBox="0 0 24 24"
      fill="none"
      stroke={strokeColor}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={cn("animate-spin", className)}
      {...props}
    >
      <path d="M21 12a9 9 0 1 1-6.219-8.56" />
    </svg>
  );
};
