export function Image({ className }: { className?: string }) {
  return (
    <svg
      width="32"
      height="32"
      viewBox="0 0 32 32"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      className={className}
    >
      <g clipPath="url(#clip0_3886_28030)">
        <g filter="url(#filter0_di_3886_28030)">
          <path
            d="M5.5 4.8C5.5 3.2536 6.7536 2 8.3 2H18.9376C19.2985 2 19.6455 2.1394 19.9061 2.38911L26.0685 8.29358C26.3441 8.55765 26.5 8.92278 26.5 9.30447V27.2C26.5 28.7463 25.2463 30 23.7 30H8.3C6.7536 30 5.5 28.7463 5.5 27.2V4.8Z"
            fill="#00C8E6"
          />
          <path
            d="M5.75 4.8C5.75 3.39167 6.89167 2.25 8.3 2.25H18.9376C19.234 2.25 19.5191 2.3645 19.7332 2.56964L25.8956 8.47407C26.1219 8.69102 26.25 8.99099 26.25 9.30447V27.2C26.25 28.6082 25.1083 29.75 23.7 29.75H8.3C6.89168 29.75 5.75 28.6082 5.75 27.2V4.8Z"
            stroke="#27272A"
            strokeOpacity="0.15"
            strokeWidth="0.5"
          />
        </g>
        <path
          d="M19.5 3.40039L25.1 9.00038H19.5V3.40039Z"
          fill="white"
          fillOpacity="0.5"
        />
        <path
          fillRule="evenodd"
          clipRule="evenodd"
          d="M14.6495 20.7714C14.6065 20.6679 14.4733 20.6394 14.3918 20.7164L11.8032 23.1607C11.698 23.2601 11.7683 23.437 11.9131 23.437H20.2081C20.3293 23.437 20.4065 23.3073 20.3488 23.2008L18.4962 19.7841C18.4461 19.6916 18.3219 19.6718 18.2455 19.7442L15.3581 22.4808L14.6495 20.7714ZM21.7593 24.0783C21.7593 24.4308 21.6337 24.7326 21.3825 24.9839C21.1312 25.2352 20.8293 25.3608 20.4769 25.3608H11.5106C11.1656 25.3608 10.8675 25.2352 10.6162 24.9839C10.365 24.7326 10.2393 24.4308 10.2393 24.0783V15.112C10.2393 14.767 10.365 14.4689 10.6162 14.2177C10.8675 13.9664 11.1656 13.8408 11.5106 13.8408H20.4769C20.8293 13.8408 21.1312 13.9645 21.3825 14.212C21.6337 14.4595 21.7593 14.7595 21.7593 15.112V24.0783Z"
          fill="white"
        />
      </g>
      <defs>
        <filter
          id="filter0_di_3886_28030"
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
            result="effect1_dropShadow_3886_28030"
          />
          <feBlend
            mode="normal"
            in="SourceGraphic"
            in2="effect1_dropShadow_3886_28030"
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
            result="effect2_innerShadow_3886_28030"
          />
        </filter>
        <clipPath id="clip0_3886_28030">
          <rect width="32" height="32" fill="white" />
        </clipPath>
      </defs>
    </svg>
  );
}
