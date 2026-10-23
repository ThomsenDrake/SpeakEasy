import type { SVGProps } from "react";

export function FlagPixelIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <svg
      width="60"
      height="70"
      viewBox="0 0 60 70"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      {...props}
    >
      <g clipPath="url(#clip0_flag)">
        <path
          d="M59.998 9.99902H29.998V20H49.998V40.005H29.998V50.005L59.998 49.999V9.99902Z"
          fill="currentColor"
        />
        <path
          d="M0 70.0011H10V40.0001H29.998V29.9991H10V9.99414H29.998V-0.00585938L0 0.000140625V70.0011Z"
          fill="currentColor"
        />
      </g>
      <defs>
        <clipPath id="clip0_flag">
          <rect width="60" height="70" fill="white" />
        </clipPath>
      </defs>
    </svg>
  );
}
