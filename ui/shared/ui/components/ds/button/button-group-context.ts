"use client";

import { createContext } from "react";

import type { ButtonStyles } from "./button-variants";

const ButtonGroupContext = createContext<{ styles: ButtonStyles } | null>(null);

export { ButtonGroupContext };
