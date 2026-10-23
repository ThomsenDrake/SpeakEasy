import { type ComponentPropsWithoutRef } from "react";

interface CrossProps extends ComponentPropsWithoutRef<"svg"> {
  size?: string;
}

export const Cross = ({ size = "28", ...props }: CrossProps) => (
  <svg
    xmlns="http://www.w3.org/2000/svg"
    width={size}
    height={size}
    viewBox="0 0 12 13"
    fill="currentColor"
    {...props}
  >
    <path fill="#FA500F" d="M7.2 4.987H4.8v2.4h2.4z" />
    <path
      fill="currentColor"
      d="M4.8 7.388H2.4v2.4h2.4zM2.4 9.787H0v2.4h2.4zM9.6 2.588H7.2v2.4h2.4zM12 .188H9.6v2.4H12zM4.8 2.588H2.4v2.4h2.4zM2.4.188H0v2.4h2.4zM12 9.787H9.6v2.4H12zM9.6 7.388H7.2v2.4h2.4z"
    />
  </svg>
);
