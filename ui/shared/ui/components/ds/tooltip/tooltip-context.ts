import { createContext } from "react";

import type { TooltipStyles } from "./tooltip-variants";

const TooltipContext = createContext<{ styles: TooltipStyles } | null>(null);

export { TooltipContext };
