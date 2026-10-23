export function Video({ className }: { className?: string }) {
  return (
    <svg
      width="32"
      height="32"
      viewBox="0 0 32 32"
      fill="none"
      className={className}
      xmlns="http://www.w3.org/2000/svg"
    >
      <g clipPath="url(#clip0_3886_28032)">
        <g filter="url(#filter0_di_3886_28032)">
          <path
            d="M5.5 4.8C5.5 3.2536 6.7536 2 8.3 2H18.9376C19.2985 2 19.6455 2.1394 19.9061 2.38911L26.0685 8.29358C26.3441 8.55765 26.5 8.92278 26.5 9.30447V27.2C26.5 28.7463 25.2463 30 23.7 30H8.3C6.7536 30 5.5 28.7463 5.5 27.2V4.8Z"
            fill="#B667E5"
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
          d="M19.1952 18.88V16.6413C19.1952 16.4687 19.1334 16.3187 19.0096 16.1913C18.8858 16.0638 18.7339 16 18.5539 16H10.8814C10.7014 16 10.5496 16.0638 10.4258 16.1913C10.3021 16.3187 10.2402 16.4687 10.2402 16.6413V23.0425C10.2402 23.2225 10.3021 23.3744 10.4258 23.4982C10.5496 23.6218 10.7014 23.6838 10.8814 23.6838H18.5539C18.7339 23.6838 18.8858 23.6218 19.0096 23.4982C19.1334 23.3744 19.1952 23.2225 19.1952 23.0425V20.8038L21.0774 22.6859C21.3294 22.9379 21.7602 22.7594 21.7602 22.403V17.289C21.7602 16.933 21.3302 16.7543 21.078 17.0054L19.1952 18.88Z"
          fill="white"
        />
      </g>
      <defs>
        <filter
          id="filter0_di_3886_28032"
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
            result="effect1_dropShadow_3886_28032"
          />
          <feBlend
            mode="normal"
            in="SourceGraphic"
            in2="effect1_dropShadow_3886_28032"
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
            result="effect2_innerShadow_3886_28032"
          />
        </filter>
        <clipPath id="clip0_3886_28032">
          <rect width="32" height="32" fill="white" />
        </clipPath>
      </defs>
    </svg>
  );
}
