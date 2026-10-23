import * as React from "react";

import { cn } from "#utils/cn.ts";

const Card = ({ className, ref, ...props }: React.ComponentProps<"div">) => (
  <div
    ref={ref}
    className={cn("bg-default text-default border shadow-2xs", className)}
    {...props}
  />
);
Card.displayName = "Card";

const CardHeader = ({
  className,
  ref,
  ...props
}: React.ComponentProps<"div">) => (
  <div
    ref={ref}
    className={cn("flex flex-col space-y-1.5 p-6", className)}
    {...props}
  />
);
CardHeader.displayName = "CardHeader";

const CardTitle = ({
  className,
  ref,
  ...props
}: React.ComponentProps<"div">) => (
  <h3
    ref={ref}
    className={cn(
      "text-2xl leading-none font-semibold tracking-tight",
      className,
    )}
    {...props}
  />
);
CardTitle.displayName = "CardTitle";

const CardDescription = ({
  className,
  ref,
  ...props
}: React.ComponentProps<"div">) => (
  <p ref={ref} className={cn("text-muted text-sm", className)} {...props} />
);
CardDescription.displayName = "CardDescription";

const CardContent = ({
  className,
  ref,
  ...props
}: React.ComponentProps<"div">) => (
  <div ref={ref} className={cn("p-6 pt-0", className)} {...props} />
);
CardContent.displayName = "CardContent";

const CardFooter = ({
  className,
  ref,
  ...props
}: React.ComponentProps<"div">) => (
  <div
    ref={ref}
    className={cn("flex items-center p-6 pt-0", className)}
    {...props}
  />
);
CardFooter.displayName = "CardFooter";

export {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
};
