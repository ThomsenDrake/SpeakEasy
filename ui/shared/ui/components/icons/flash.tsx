import type { ComponentPropsWithoutRef, ElementType } from "react";

interface FlashProps extends ComponentPropsWithoutRef<"svg"> {
  size?: string;
  asChild?: boolean;
  as?: ElementType;
  fill?: string;
}

export const Flash: React.FC<FlashProps> = ({
  size = "16",
  asChild = false,
  as,
  fill = "#FFAF00",
  ...props
}) => {
  const Component = asChild ? as || "svg" : "svg";

  return (
    <Component
      width={size}
      height={size}
      viewBox="0 0 16 16"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      {...props}
    >
      <g clipPath="url(#clip0_964_11673)">
        <path d="M11.4284 0H9.14264V2.28571H11.4284V0Z" fill={fill} />
        <path
          d="M13.7138 6.85636H9.14263V2.28516H6.85692V4.57087H4.57121V6.85636H2.28526V9.14207H6.85692V13.7137H4.57121V15.9994H6.85692V13.7137H9.14263V11.4282H11.4283V9.14207H13.7138V6.85636Z"
          fill={fill}
        />
      </g>
      <defs>
        <clipPath id="clip0_964_11673">
          <rect
            width="11.4286"
            height="16"
            fill="white"
            transform="translate(2.28572)"
          />
        </clipPath>
      </defs>
    </Component>
  );
};
