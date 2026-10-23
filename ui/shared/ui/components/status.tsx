import * as React from "react";

import { cn } from "../utils/cn";

interface StatusProps extends React.ComponentProps<"div"> {
  type?: "info" | "danger";
}

const Status = ({ className, type = "info", ref, ...props }: StatusProps) => (
  <div
    ref={ref}
    role="status"
    className={cn(
      "rounded-md border-l-4 px-6 py-5",
      className,
      type === "danger"
        ? "border-(--red-500) bg-(--red-50) text-(--red-500)"
        : "border-sky-700 bg-sky-50 text-sky-700",
    )}
    {...props}
  />
);
Status.displayName = "Status";

const StatusTitle = ({
  className,
  ref,
  ...props
}: React.ComponentProps<"h5">) => (
  <h5
    ref={ref}
    className={cn("mb-2 leading-none font-semibold tracking-tight", className)}
    {...props}
  />
);
StatusTitle.displayName = "StatusTitle";

const StatusDescription = ({
  className,
  ref,
  ...props
}: React.ComponentProps<"div">) => (
  <div
    ref={ref}
    className={cn("text-sm [&_p]:leading-relaxed", className)}
    {...props}
  />
);
StatusDescription.displayName = "StatusDescription";

export { Status, StatusDescription, StatusTitle };
