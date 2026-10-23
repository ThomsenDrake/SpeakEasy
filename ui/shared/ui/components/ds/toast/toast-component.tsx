"use client";

import type { LucideIcon } from "lucide-react";
import {
  BellDot,
  CircleCheck,
  LoaderCircle,
  TriangleAlert,
  X,
} from "lucide-react";
import React, { createElement, useMemo } from "react";
import { match, P } from "ts-pattern";

import type { ButtonProps } from "../button";
import { Button } from "../button";
import { Typography } from "../typography";
import { ToastContext } from "./toast-context";
import { toastVariant } from "./toast-variant";

type ToastVariant = "success" | "error" | "warning" | "info" | "loading";

const iconMapping: Record<ToastVariant, LucideIcon | null> = {
  success: CircleCheck,
  error: TriangleAlert,
  warning: BellDot,
  loading: LoaderCircle,
  info: null,
};

type ToastAction = ButtonProps | React.ReactNode;
export type ToastProps = {
  title: React.ReactNode;
  description?: React.ReactNode;
  actions?: [ToastAction] | [ToastAction, ToastAction];
  variant?: ToastVariant;
  className?: string;
};

type InternalToastProps = ToastProps & {
  close?: VoidFunction;
  isDismissible?: boolean;
};

export const Toast = ({
  title,
  description,
  actions,
  variant,
  className,
  isDismissible,
  close,
  ...props
}: InternalToastProps) => {
  const isOneLiner = !description && !isDismissible;

  const contextValue = useMemo(
    () => ({
      styles: toastVariant({ variant, isDismissible, isOneLiner }),
    }),
    [isDismissible, variant, isOneLiner],
  );

  const IconComponent = variant ? iconMapping[variant] : null;

  const hasActions = !!actions?.length;

  const Title = match(title)
    .with(P.string, (title) => (
      <Typography size="sm" weight="semibold" className="line-clamp-4 truncate">
        {title}
      </Typography>
    ))
    .otherwise((title) => title);

  const Description = match(description || null)
    .with(P.nullish, () => null)
    .with(P.string, (description) => (
      <Typography size="sm" variant="muted" className="line-clamp-4 truncate">
        {description}
      </Typography>
    ))
    .otherwise((description) => description);

  return (
    <ToastContext.Provider value={contextValue}>
      <div className={contextValue.styles.base({ className })} {...props}>
        {IconComponent ? (
          <div className={contextValue.styles.icon()}>
            {createElement(IconComponent, {
              className: contextValue.styles.icon(),
            })}
          </div>
        ) : null}
        <div className={contextValue.styles.content()}>
          <div className={contextValue.styles.text_container()}>
            {Title}
            {Description}
          </div>
          {hasActions ? (
            <div className={contextValue.styles.buttonContainer()}>
              {actions.map((action, idx) =>
                match(action)
                  .with(P.instanceOf(Object), (actionProps) => (
                    <Button
                      key={idx.toString()}
                      size="xs"
                      variant={idx > 0 ? "soft" : "primary"}
                      {...(actionProps as ButtonProps)}
                      onClick={(e) => {
                        close?.();
                        (actionProps as ButtonProps).onClick?.(e);
                      }}
                    />
                  ))
                  .otherwise((reactNode) => reactNode),
              )}
            </div>
          ) : null}
        </div>
        {isDismissible ? (
          <div className={contextValue.styles.closeContainer()}>
            <Button
              onClick={close}
              mode="icon-only"
              size="xs"
              shape="rounded"
              className={contextValue.styles.closeButton()}
              variant="ghost-muted"
              icon={X}
              data-testid="toast-dismiss-button"
            />
          </div>
        ) : null}
      </div>
    </ToastContext.Provider>
  );
};
