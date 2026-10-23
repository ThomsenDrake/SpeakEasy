import { createContext } from "react";

import type { InputVariantProps, InputVariantStyles } from "./input-variants";

type InputContainerContextValue = InputVariantStyles;

const InputContainerContext = createContext<{
  styles: InputContainerContextValue;
  variants: InputVariantProps;
} | null>(null);

export { InputContainerContext };
