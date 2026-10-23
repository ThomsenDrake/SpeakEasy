import * as React from "react";

import { cn } from "#utils/cn.ts";

/**
 * @deprecated ⚠️ Use `Input` from `@mistral/ui/input` instead ⚠️
 */
const Input = ({
  className,
  type,
  ref,
  ...props
}: React.ComponentProps<"input"> & { ref?: React.Ref<HTMLInputElement> }) => {
  return (
    <input
      type={type}
      className={cn(
        "border-default bg-default ring-offset-background file:text-default placeholder:text-muted focus-visible:ring-default flex h-10 w-full rounded-md border px-3 py-2 text-base file:border-0 file:bg-transparent file:text-sm file:font-medium focus-visible:ring-2 focus-visible:ring-offset-2 focus-visible:outline-hidden disabled:cursor-not-allowed disabled:opacity-50 md:text-sm",
        className,
      )}
      ref={ref}
      {...props}
    />
  );
};

Input.displayName = "Input";

export { Input };
