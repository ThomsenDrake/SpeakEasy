"use client";

import { createContext } from "react";

import type { ToastStyles } from "./toast-variant";

const ToastContext = createContext<{ styles: ToastStyles } | null>(null);

export { ToastContext };
