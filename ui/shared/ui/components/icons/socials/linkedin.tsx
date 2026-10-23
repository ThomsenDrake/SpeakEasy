import type { SVGProps } from "react";

import { cn } from "#utils/cn.ts";

export const LinkedInIcon = (props: SVGProps<SVGSVGElement>) => {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      width="20"
      height="20"
      {...props}
      className={cn("text-icon-default", props.className)}
    >
      <path
        fill="currentColor"
        d="M5.784 4.167a1.667 1.667 0 1 1-3.334-.002 1.667 1.667 0 0 1 3.334.002Zm.05 2.9H2.5V17.5h3.334V7.067Zm5.266 0H7.784V17.5h3.283v-5.475c0-3.05 3.975-3.333 3.975 0V17.5h3.292v-6.608c0-5.142-5.884-4.95-7.267-2.425l.033-1.4Z"
      />
    </svg>
  );
};
