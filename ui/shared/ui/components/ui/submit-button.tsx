import { Loader2 } from "lucide-react";

import { cn } from "#utils/cn.ts";

import { Button } from "../ds/button";
import { ArrowRight } from "../icons/arrow-right";

export const SubmitButton = ({
  className,
  isLoading,
  children,
  ...props
}: {
  className?: string;
  isDisabled?: boolean;
  isLoading?: boolean;
  children?: React.ReactNode;
} & (
  | { onClick: () => void }
  | { onSubmit: () => void }
  | { type?: "submit" }
)) => {
  return (
    <Button
      className={cn("shadow-none inset-shadow-none", className)}
      variant="primary"
      aria-label="Send question"
      mode="icon-only"
      icon={
        isLoading ? (
          <Loader2 className="animate-spin" size="16" />
        ) : (
          <ArrowRight className="-rotate-90" size="14" />
        )
      }
      {...props}
    >
      {children}
    </Button>
  );
};
