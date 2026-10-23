"use client";

import { createContext } from "react";

import type { ButtonStyles } from "./button-variants";

const ButtonContext = createContext<{ styles: ButtonStyles } | null>(null);

export { ButtonContext };
