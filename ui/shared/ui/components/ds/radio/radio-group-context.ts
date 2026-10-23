import type { ChangeEventHandler } from "react";
import { createContext } from "react";

import type { RadioStyles } from "./radio-variants";

const RadioGroupContext = createContext<{
  styles: RadioStyles;
  isDisabled?: boolean;
  value: string | undefined;
  onValueChange: (value: string) => void;
  onChange?: ChangeEventHandler<HTMLInputElement>;
} | null>(null);

export { RadioGroupContext };
