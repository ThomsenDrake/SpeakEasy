"use client";

import { createContext } from "react";

import type { CommandStyles, CommandVariantProps } from "./command-variants";

const CommandContext = createContext<{
  styles: CommandStyles;
  variants: CommandVariantProps;
} | null>(null);

export { CommandContext };
