import { type ComponentPropsWithoutRef } from "react";

interface ArrowRightProps extends ComponentPropsWithoutRef<"svg"> {
  size?: string;
}

export const ArrowRight = ({ size = "28", ...props }: ArrowRightProps) => {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      id="a"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="currentColor"
      version="1.1"
      {...props}
    >
      <path
        fill="currentColor"
        d="M12 18v4h4v-4h-4ZM16 14v4h4v-4h-4ZM20 10v4h4v-4h-4ZM16 6v4h4V6h-4ZM12 2v4h4V2h-4ZM12 10v4h4v-4h-4ZM8 10v4h4v-4H8ZM4 10v4h4v-4H4ZM0 10v4h4v-4H0Z"
      />
    </svg>
  );
};
