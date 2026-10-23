export function Spreadsheet({ className }: { className?: string }) {
  return (
    <svg
      width="32"
      height="32"
      viewBox="0 0 32 32"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      className={className}
    >
      <g clipPath="url(#clip0_7274_3765)">
        <g filter="url(#filter0_di_7274_3765)">
          <path
            d="M5.5 4.8C5.5 3.2536 6.7536 2 8.3 2H18.9376C19.2985 2 19.6455 2.1394 19.9061 2.38911L26.0685 8.29358C26.3441 8.55765 26.5 8.92278 26.5 9.30447V27.2C26.5 28.7463 25.2463 30 23.7 30H8.3C6.7536 30 5.5 28.7463 5.5 27.2V4.8Z"
            fill="#079455"
          />
          <path
            d="M5.75 4.8C5.75 3.39167 6.89167 2.25 8.3 2.25H18.9376C19.234 2.25 19.5191 2.3645 19.7332 2.56964L25.8956 8.47407C26.1219 8.69102 26.25 8.99099 26.25 9.30447V27.2C26.25 28.6082 25.1083 29.75 23.7 29.75H8.3C6.89168 29.75 5.75 28.6082 5.75 27.2V4.8Z"
            stroke="#27272A"
            strokeOpacity="0.15"
            strokeWidth="0.5"
          />
        </g>
        <g clipPath="url(#clip1_7274_3765)">
          <path
            d="M10.8159 19.5761H21.1839M10.8159 19.5761V17.2721C10.8159 16.6359 11.3317 16.1201 11.9679 16.1201H14.2719M10.8159 19.5761V21.8801C10.8159 22.5163 11.3317 23.0321 11.9679 23.0321H14.2719M21.1839 19.5761V21.8801C21.1839 22.5163 20.6682 23.0321 20.0319 23.0321H14.2719M21.1839 19.5761V17.2721C21.1839 16.6359 20.6682 16.1201 20.0319 16.1201H14.2719M14.2719 16.1201V23.0321"
            stroke="white"
            strokeWidth="1.08"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </g>
        <path
          d="M19.5 3.40039L25.1 9.00038H19.5V3.40039Z"
          fill="white"
          fillOpacity="0.5"
        />
      </g>
      <defs>
        <filter
          id="filter0_di_7274_3765"
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
            result="effect1_dropShadow_7274_3765"
          />
          <feBlend
            mode="normal"
            in="SourceGraphic"
            in2="effect1_dropShadow_7274_3765"
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
            result="effect2_innerShadow_7274_3765"
          />
        </filter>
        <clipPath id="clip0_7274_3765">
          <rect width="32" height="32" fill="white" />
        </clipPath>
        <clipPath id="clip1_7274_3765">
          <rect
            width="11.52"
            height="11.52"
            fill="white"
            transform="translate(10.2397 13.2402)"
          />
        </clipPath>
      </defs>
    </svg>
  );
}
