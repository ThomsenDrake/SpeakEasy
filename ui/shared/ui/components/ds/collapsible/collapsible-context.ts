"use client";

import { createContext, useContext } from "react";

import type { CollapsibleSize } from "./collapsible";

interface CollapsibleContextValue {
  isOpen: boolean;
  toggle: () => void;
  contentId: string;
  triggerId: string;
  size: CollapsibleSize;
  fullWidth?: boolean;
}

const CollapsibleContext = createContext<CollapsibleContextValue | null>(null);

const useCollapsibleContext = () => {
  const context = useContext(CollapsibleContext);
  if (!context) {
    throw new Error(
      "Collapsible compound components must be used within a Collapsible",
    );
  }

  return context;
};

export type { CollapsibleContextValue };
export { CollapsibleContext, useCollapsibleContext };
