"use client";

import { motion } from "framer-motion";
import React from "react";

interface MistralLogoProps {
  width?: number;
  height?: number;
  className?: string;
}

const MistralLogo: React.FC<MistralLogoProps> = ({
  width = 393.939,
  height = 333.333,
  className = "",
}) => {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      width={width}
      height={height}
      viewBox="0 0 393.939 333.333"
      className={className}
    >
      <rect
        x="121.212"
        y="90.909"
        width="30.303"
        height="30.303"
        fill="gold"
        strokeWidth="0"
      />
      <rect
        x="242.424"
        y="90.909"
        width="30.303"
        height="30.303"
        fill="gold"
        strokeWidth="0"
      />
      <rect
        x="121.212"
        y="121.212"
        width="60.606"
        height="30.303"
        fill="#ffaf00"
        strokeWidth="0"
      />
      <rect
        x="212.121"
        y="121.212"
        width="60.606"
        height="30.303"
        fill="#ffaf00"
        strokeWidth="0"
      />
      <rect
        x="121.212"
        y="151.515"
        width="151.515"
        height="30.303"
        fill="#ff8205"
        strokeWidth="0"
      />
      <rect
        x="121.212"
        y="181.818"
        width="30.303"
        height="30.303"
        fill="#fa500f"
        strokeWidth="0"
      />
      <rect
        x="181.818"
        y="181.818"
        width="30.303"
        height="30.303"
        fill="#fa500f"
        strokeWidth="0"
      />
      <rect
        x="242.424"
        y="181.818"
        width="30.303"
        height="30.303"
        fill="#fa500f"
        strokeWidth="0"
      />
      <rect
        x="90.909"
        y="212.121"
        width="90.909"
        height="30.303"
        fill="#e10500"
        strokeWidth="0"
      />
      <rect
        x="212.121"
        y="212.121"
        width="90.909"
        height="30.303"
        fill="#e10500"
        strokeWidth="0"
      />
    </svg>
  );
};

export const Spinner = ({ size = 100 }: { size?: number }) => {
  return (
    <div
      className="relative flex items-center justify-center"
      style={{ width: size, height: size }}
    >
      <div
        className="absolute rounded-full bg-white/10"
        style={{
          width: size,
          height: size,
        }}
      />

      <motion.div
        className="absolute"
        style={{
          width: size,
          height: size,
        }}
        animate={{
          rotate: -1080,
        }}
        transition={{
          duration: 2,
          ease: [0.5, 0.2, 0.15, 0.9], // Custom bezier curve
          repeat: Infinity,
          repeatType: "loop", // Ensures smooth looping without pauses
        }}
      >
        <svg
          width={size}
          height={size}
          viewBox={`0 0 ${size} ${size}`}
          className="absolute top-0 left-0"
        >
          <defs>
            <linearGradient id="tailGradient" x1="0%" y1="0%" x2="100%" y2="0%">
              <stop offset="0%" stopColor="#C7C7CB" stopOpacity="1" />
              <stop offset="50%" stopColor="#C7C7CB" stopOpacity="0.7" />
              <stop offset="75%" stopColor="#C7C7CB" stopOpacity="0.5" />
              <stop offset="100%" stopColor="#C7C7CB" stopOpacity="0" />
            </linearGradient>
            <mask id="tailMask">
              <path
                d={`M ${size / 2} ${size * 0.08} A ${size * 0.42} ${size * 0.42} 0 0 1 ${size / 2} ${size * 0.92}`}
                stroke="white"
                strokeWidth={size * 0.08}
                fill="none"
                strokeDasharray={`${size * 0.75} ${size * 1.25}`}
                strokeLinecap="round"
              />
            </mask>
          </defs>
          <circle
            cx={size / 2}
            cy={size / 2}
            r={size * 0.42}
            stroke="url(#tailGradient)"
            strokeWidth={size * 0.1}
            fill="none"
            mask="url(#tailMask)"
            strokeLinecap="round"
          />
        </svg>
      </motion.div>

      <div
        className="relative z-10"
        style={{
          width: size,
          height: size,
        }}
      >
        <MistralLogo width={size} height={size} />
      </div>
    </div>
  );
};

export default Spinner;
