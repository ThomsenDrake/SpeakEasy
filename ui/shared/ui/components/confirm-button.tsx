"use client";

import * as React from "react";
import { useEffect, useRef, useState } from "react";

import { cn } from "../utils/cn";
import type { ButtonProps } from "./ds/button/button";
import { Button } from "./ds/button/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "./ds/tooltip";

export interface WithConfirmProps {
  onConfirm: () => void;
  confirmText?: string | React.ReactNode;
  confirmTooltip?: string;
  confirmVariant?: ButtonProps["variant"];
  tooltip?: string;
}

// Omit mode and icon from ButtonProps to avoid type conflicts
export interface ConfirmButtonProps
  extends Omit<ButtonProps, "onClick" | "mode" | "icon">, WithConfirmProps {}

const ConfirmButton = ({
  className,
  variant = "primary",
  confirmVariant = "destructive",
  confirmText,
  confirmTooltip,
  tooltip,
  onConfirm,
  children,
  ...props
}: ConfirmButtonProps) => {
  const [isWaitingForConfirm, setIsWaitingForConfirm] = useState(false);
  const buttonRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!isWaitingForConfirm) return;

    const handleClickOutside = (event: MouseEvent | TouchEvent) => {
      if (
        buttonRef.current &&
        !buttonRef.current.contains(event.target as Node)
      ) {
        setIsWaitingForConfirm(false);
      }
    };

    document.addEventListener("mousedown", handleClickOutside);
    document.addEventListener("touchstart", handleClickOutside);

    return () => {
      document.removeEventListener("mousedown", handleClickOutside);
      document.removeEventListener("touchstart", handleClickOutside);
    };
  }, [isWaitingForConfirm]);

  const handleClick = () => {
    if (!isWaitingForConfirm) {
      setIsWaitingForConfirm(true);
      return;
    }

    onConfirm();
    setIsWaitingForConfirm(false);
  };

  const handleMouseLeave = () => {
    setIsWaitingForConfirm(false);
  };

  const getConfirmTextColor = (variant: ButtonProps["variant"]) => {
    switch (variant) {
      case "destructive":
      case "primary":
      case "brand":
        return "!text-inverted-default";
      case "secondary":
      case "dashed":
      case "soft":
      case "ghost":
      case "ghost-muted":
      case "none":
      case undefined:
      default:
        return "!text-default";
    }
  };

  return (
    <Tooltip open={isWaitingForConfirm}>
      <TooltipTrigger asChild>
        <Button
          variant={isWaitingForConfirm ? confirmVariant : variant}
          onMouseLeave={handleMouseLeave}
          onClick={handleClick}
          className={cn(
            className,
            isWaitingForConfirm && getConfirmTextColor(confirmVariant),
          )}
          mode="default"
          {...props}
        >
          {isWaitingForConfirm ? (confirmText ?? "Confirm") : children}
        </Button>
      </TooltipTrigger>
      <TooltipContent>
        <p>
          {isWaitingForConfirm
            ? (confirmTooltip ?? "Click again to confirm")
            : tooltip}
        </p>
      </TooltipContent>
    </Tooltip>
  );
};
ConfirmButton.displayName = "ConfirmButton";

export { ConfirmButton };
