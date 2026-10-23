import { createContext } from "react";

import type { CheckboxStyles } from "./checkbox-variants";

const CheckboxContainerContext = createContext<{
  styles: CheckboxStyles;
  id: string;
} | null>(null);

export { CheckboxContainerContext };
