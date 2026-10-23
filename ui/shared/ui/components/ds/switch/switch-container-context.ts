import { createContext } from "react";

import type { SwitchStyles } from "./switch-variants";

const SwitchContainerContext = createContext<{
  styles: SwitchStyles;
  id: string;
} | null>(null);

export { SwitchContainerContext };
