import { createContext } from "react";

import type { FormFieldStyles } from "./form-field-variants";

type FieldContextType = {
  id: string;
  name: string;
  isInvalid: boolean;
  styles: FormFieldStyles;
};

export const FieldContext = createContext<FieldContextType | undefined>(
  undefined,
);
