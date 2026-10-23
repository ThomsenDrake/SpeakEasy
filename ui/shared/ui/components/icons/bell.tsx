import type { SVGProps } from "react";

export function BellPixelIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <svg
      width="70"
      height="70"
      viewBox="0 0 70 70"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      {...props}
    >
      <g clipPath="url(#clip0_bell)">
        <path d="M39.999 0H29.999V10H39.999V0Z" fill="currentColor" />
        <path
          d="M29.999 9.99902H19.999V19.999H29.999V9.99902Z"
          fill="currentColor"
        />
        <path
          d="M59.998 20L49.999 19.999L49.998 49.998H50V50L19.998 49.999V20H9.99905L9.99805 49.999L-0.000953125 49.998L-0.00195312 59.999H30H29.999V70H30H40V59.999L69.998 60L69.999 50H60L59.998 20Z"
          fill="currentColor"
        />
        <path
          d="M49.999 9.99902H39.999V19.999H49.999V9.99902Z"
          fill="currentColor"
        />
      </g>
      <defs>
        <clipPath id="clip0_bell">
          <rect width="70" height="70" fill="white" />
        </clipPath>
      </defs>
    </svg>
  );
}
