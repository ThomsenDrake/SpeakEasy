import type { SVGProps } from "react";

import { cn } from "#utils/cn.ts";

export const MailIcon = (props: SVGProps<SVGSVGElement>) => {
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
        d="M2.7 2.5h15c.46 0 .833.373.833.833v13.334c0 .46-.373.833-.833.833h-15a.833.833 0 0 1-.833-.833V3.333c0-.46.373-.833.833-.833Zm14.167 3.532-6.607 5.916-6.727-5.935v9.82h13.334V6.032ZM3.96 4.167l6.292 5.551 6.199-5.551H3.96Z"
      />
    </svg>
  );
};
