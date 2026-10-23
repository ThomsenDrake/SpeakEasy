import type { SVGProps } from "react";

import { cn } from "#utils/cn.ts";

export const XIcon = (props: SVGProps<SVGSVGElement>) => {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      width="21"
      height="20"
      {...props}
      className={cn("text-icon-default", props.className)}
    >
      <path
        fill="currentColor"
        d="M7.267 1.667H1.433l6.884 9.178-6.509 7.488h2.209L9.34 12.21l4.593 6.124h5.834L12.593 8.77l6.174-7.102h-2.209L11.57 7.405 7.267 1.667Zm7.5 15-10-13.334h1.666l10 13.334h-1.666Z"
      />
    </svg>
  );
};
