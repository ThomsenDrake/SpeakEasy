import { type ComponentPropsWithoutRef } from "react";

interface CheckProps extends ComponentPropsWithoutRef<"svg"> {
  size?: string;
  fill?: string;
}

export const Check = ({
  size = "24",
  fill = "#FF8205",
  ...props
}: CheckProps) => (
  <svg
    xmlns="http://www.w3.org/2000/svg"
    width={size}
    height={size}
    fill="currentColor"
    viewBox="0 0 24 24"
    {...props}
  >
    <path fill={fill} d="M10.6 15.657H7.792v2.81H10.6z" />
    <path
      fill={fill}
      d="M19 7.334h-2.808v2.808h-2.808v2.809h-2.808v2.809h2.808V12.95h2.808v-2.809H19zM7.792 12.849H4.985v2.809h2.807z"
    />
  </svg>
);

export default Check;
