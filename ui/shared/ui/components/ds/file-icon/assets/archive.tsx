export function Archive({ className }: { className?: string }) {
  return (
    <svg
      width="32"
      height="32"
      viewBox="0 0 32 32"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      className={className}
    >
      <g clipPath="url(#clip0_3886_28029)">
        <g filter="url(#filter0_di_3886_28029)">
          <path
            d="M5.5 4.8C5.5 3.2536 6.7536 2 8.3 2H18.9376C19.2985 2 19.6455 2.13941 19.9061 2.38912L26.0685 8.29358C26.3441 8.55765 26.5 8.92278 26.5 9.30449V27.2C26.5 28.7463 25.2463 30 23.7 30H8.3C6.7536 30 5.5 28.7463 5.5 27.2V4.8Z"
            fill="white"
          />
          <path
            d="M5.75 4.8C5.75 3.39167 6.89167 2.25 8.3 2.25H18.9376C19.234 2.25 19.519 2.36451 19.7332 2.56965L25.8956 8.47407C26.1219 8.69102 26.25 8.99098 26.25 9.30449V27.2C26.25 28.6082 25.1083 29.75 23.7 29.75H8.3C6.89168 29.75 5.75 28.6082 5.75 27.2V4.8Z"
            stroke="#27272A"
            strokeOpacity="0.15"
            strokeWidth="0.5"
          />
        </g>
        <path
          d="M15.1348 18.7363H17.1508V19.6003H15.1348V18.7363Z"
          fill="#A1A1A9"
        />
        <path
          d="M15.1348 15.2803H17.1508V16.1443H15.1348V15.2803Z"
          fill="#A1A1A9"
        />
        <path
          d="M15.1348 12.4004H17.1508V13.2644H15.1348V12.4004Z"
          fill="#A1A1A9"
        />
        <path
          d="M17.0074 21.3281H14.9914C14.9914 22.6961 13.9834 24.0641 13.9834 24.9761C13.9834 25.9842 14.8852 26.8001 15.9994 26.8001C17.1137 26.8001 18.0154 25.9842 18.0154 24.9761C18.0154 24.0641 17.0074 22.6961 17.0074 21.3281ZM15.9994 25.8881C15.4423 25.8881 14.9914 25.4802 14.9914 24.9761C14.9914 24.472 15.4423 24.0641 15.9994 24.0641C16.5566 24.0641 17.0074 24.472 17.0074 24.9761C17.0074 25.4802 16.5566 25.8881 15.9994 25.8881Z"
          fill="#A1A1A9"
        />
      </g>
      <defs>
        <filter
          id="filter0_di_3886_28029"
          x="4"
          y="1.5"
          width="24"
          height="31"
          filterUnits="userSpaceOnUse"
          colorInterpolationFilters="sRGB"
        >
          <feFlood floodOpacity="0" result="BackgroundImageFix" />
          <feColorMatrix
            in="SourceAlpha"
            type="matrix"
            values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0"
            result="hardAlpha"
          />
          <feOffset dy="1" />
          <feGaussianBlur stdDeviation="0.75" />
          <feComposite in2="hardAlpha" operator="out" />
          <feColorMatrix
            type="matrix"
            values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0.06 0"
          />
          <feBlend
            mode="normal"
            in2="BackgroundImageFix"
            result="effect1_dropShadow_3886_28029"
          />
          <feBlend
            mode="normal"
            in="SourceGraphic"
            in2="effect1_dropShadow_3886_28029"
            result="shape"
          />
          <feColorMatrix
            in="SourceAlpha"
            type="matrix"
            values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0"
            result="hardAlpha"
          />
          <feOffset dy="-0.5" />
          <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1" />
          <feColorMatrix
            type="matrix"
            values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0.1 0"
          />
          <feBlend
            mode="normal"
            in2="shape"
            result="effect2_innerShadow_3886_28029"
          />
        </filter>
        <clipPath id="clip0_3886_28029">
          <rect width="32" height="32" fill="white" />
        </clipPath>
      </defs>
    </svg>
  );
}
