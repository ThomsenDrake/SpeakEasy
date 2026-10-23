"use client";

import React from "react";
import type { ExternalToast } from "sonner";
import { toast as sonnerToast } from "sonner";
import { match, P } from "ts-pattern";

import { Toast, type ToastProps } from "./toast-component";

export type ToastConfiguration = Pick<
  ExternalToast,
  "duration" | "onDismiss" | "onAutoClose" | "id" | "dismissible" | "toasterId"
>;

const innerToast = (
  props: ToastProps | string,
  toastOptions?: ToastConfiguration,
  intent?: "success" | "error" | "warning" | "info" | "loading",
) => {
  const componentProps = match(props)
    .with(P.string, (title) => ({ title }))
    .with(P.instanceOf(Object), (props) => props)
    .exhaustive();

  const variantOverride = typeof props !== "string" ? props.variant : undefined;

  return sonnerToast.custom(
    (id) => (
      <Toast
        {...componentProps}
        variant={variantOverride ?? intent}
        isDismissible={!!toastOptions?.dismissible}
        close={() => sonnerToast.dismiss(id)}
      />
    ),
    toastOptions,
  );
};

export const toast = (
  props: ToastProps | string,
  toastOptions?: ToastConfiguration,
) => innerToast(props, toastOptions, "info");

toast.success = (
  props: Omit<ToastProps, "variant"> | string,
  toastOptions?: ToastConfiguration,
) => innerToast(props, toastOptions, "success");

toast.error = (
  props: Omit<ToastProps, "variant"> | string,
  toastOptions?: ToastConfiguration,
) => innerToast(props, toastOptions, "error");

toast.warning = (
  props: Omit<ToastProps, "variant"> | string,
  toastOptions?: ToastConfiguration,
) => innerToast(props, toastOptions, "warning");

toast.info = (
  props: Omit<ToastProps, "variant"> | string,
  toastOptions?: ToastConfiguration,
) => innerToast(props, toastOptions, "info");

toast.loading = (
  props: Omit<ToastProps, "variant"> | string,
  toastOptions?: ToastConfiguration,
) => innerToast(props, toastOptions, "loading");

toast.dismiss = (id: string) => sonnerToast.dismiss(id);
