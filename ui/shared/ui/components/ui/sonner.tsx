"use client";

import { useTheme } from "next-themes";
import { Toaster as Sonner } from "sonner";

type ToasterProps = React.ComponentProps<typeof Sonner>;

const Toaster = ({ ...props }: ToasterProps) => {
  const { theme = "system" } = useTheme();
  return (
    <Sonner
      theme={theme as ToasterProps["theme"]}
      className="toaster group !border-none !bg-none"
      toastOptions={{
        classNames: {
          toast:
            "group toast group-[.toaster]:bg-default group-[.toaster]:text-default group-[.toaster]:border-default group-[.toaster]:shadow-lg",
          description: "group-[.toast]:text-muted",
          actionButton:
            "group-[.toast]:bg-state-brand group-[.toast]:text-inverted-default-default",
          cancelButton: "group-[.toast]:bg-muted group-[.toast]:text-muted",
        },
      }}
      {...props}
    />
  );
};

export { Toaster };
