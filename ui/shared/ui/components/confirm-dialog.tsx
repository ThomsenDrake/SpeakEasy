"use client";

import type { ReactNode } from "react";
import React, { useMemo, useRef, useState } from "react";

import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "#components/ui/alert-dialog.tsx";

import { Button } from "./ds/button";

interface ConfirmDialogContextType {
  confirm: (props: ConfirmDialogOptions) => void;
}

const ConfirmDialogContext = React.createContext<
  ConfirmDialogContextType | undefined
>(undefined);

interface ConfirmDialogOptions {
  title: ReactNode;
  description: ReactNode;
  cancel?: {
    label?: string;
    onClick?: () => void;
  };
  confirm: {
    label?: string;
    onClick: () => void | Promise<unknown>;
    isDestructive?: boolean;
  };
}

interface ConfirmDialogProviderProps {
  children: React.ReactNode;
  defaultLabels: {
    cancel: string;
    confirm: string;
  };
}

export const ConfirmDialogProvider = (props: ConfirmDialogProviderProps) => {
  const [isOpen, setIsOpen] = useState(false);
  const [options, setOptions] = useState<ConfirmDialogOptions | null>(null);
  const confirmedRef = useRef(false);

  const [isConfirmLoading, setIsConfirmLoading] = useState(false);

  const contextValue = useMemo(
    () => ({
      confirm: (options: ConfirmDialogOptions) => {
        confirmedRef.current = false;
        setOptions(options);
        setIsOpen(true);
      },
    }),
    [],
  );

  const handleOpenChange = (open: boolean) => {
    if (!open && !confirmedRef.current) {
      options?.cancel?.onClick?.();
    }
    confirmedRef.current = false;
    setIsOpen(open);
  };

  return (
    <ConfirmDialogContext.Provider value={contextValue}>
      {props.children}
      <AlertDialog open={isOpen} onOpenChange={handleOpenChange}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle className="min-h-7">
              {options?.title}
            </AlertDialogTitle>
            <AlertDialogDescription className="min-h-7">
              {options?.description}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel asChild>
              <Button variant="secondary">
                {options?.cancel?.label ?? props.defaultLabels.cancel}
              </Button>
            </AlertDialogCancel>

            <Button
              onClick={async () => {
                setIsConfirmLoading(true);
                try {
                  await options?.confirm.onClick();
                } catch {
                  return;
                } finally {
                  setIsConfirmLoading(false);
                }
                confirmedRef.current = true;
                setIsOpen(false);
              }}
              variant={
                options?.confirm.isDestructive ? "destructive" : "primary"
              }
              isLoading={isConfirmLoading}
              autoFocus
            >
              {options?.confirm.label ?? props.defaultLabels.confirm}
            </Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </ConfirmDialogContext.Provider>
  );
};

export const useConfirmDialog = () => {
  const context = React.useContext(ConfirmDialogContext);

  if (!context) {
    throw new Error(
      "useConfirmDialog must be used within a ConfirmDialogProvider",
    );
  }

  return context;
};
