export function Web({ className }: { className?: string }) {
  return (
    <svg
      width="32"
      height="32"
      viewBox="0 0 32 32"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      className={className}
    >
      <g clipPath="url(#clip0_7258_8)">
        <g filter="url(#filter0_di_7258_8)">
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
          d="M16 25C12.6863 25 10 22.3137 10 19C10 15.6863 12.6863 13 16 13C19.3137 13 22 15.6863 22 19C22 22.3137 19.3137 25 16 25ZM14.626 23.6004C14.0485 22.3755 13.6944 21.0245 13.6164 19.6H11.2371C11.4748 21.5059 12.8298 23.0648 14.626 23.6004ZM14.8184 19.6C14.9087 21.0633 15.3267 22.4378 16 23.6512C16.6733 22.4378 17.0913 21.0633 17.1816 19.6H14.8184ZM20.7629 19.6H18.3836C18.3056 21.0245 17.9516 22.3755 17.374 23.6004C19.1702 23.0648 20.5252 21.5059 20.7629 19.6ZM11.2371 18.4H13.6164C13.6944 16.9755 14.0485 15.6244 14.626 14.3995C12.8298 14.9352 11.4748 16.4941 11.2371 18.4ZM14.8184 18.4H17.1816C17.0913 16.9367 16.6733 15.5622 16 14.3488C15.3267 15.5622 14.9087 16.9367 14.8184 18.4ZM17.374 14.3995C17.9516 15.6244 18.3056 16.9755 18.3836 18.4H20.7629C20.5252 16.4941 19.1702 14.9352 17.374 14.3995Z"
          fill="#27272A"
          fillOpacity="0.25"
        />
      </g>
      <defs>
        <filter
          id="filter0_di_7258_8"
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
            result="effect1_dropShadow_7258_8"
          />
          <feBlend
            mode="normal"
            in="SourceGraphic"
            in2="effect1_dropShadow_7258_8"
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
            result="effect2_innerShadow_7258_8"
          />
        </filter>
        <clipPath id="clip0_7258_8">
          <rect width="32" height="32" fill="white" />
        </clipPath>
      </defs>
    </svg>
  );
}
