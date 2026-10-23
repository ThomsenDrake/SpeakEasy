import { createContext } from "react";

import type { RadioStyles } from "./radio-variants";

const RadioContainerContext = createContext<{
  styles: RadioStyles;
  id: string;
} | null>(null);

export { RadioContainerContext };
