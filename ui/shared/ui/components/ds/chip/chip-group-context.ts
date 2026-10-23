import { createContext } from "react";

import type { ChipStyles } from "./chip-variants";

const ChipGroupContext = createContext<{ styles: ChipStyles } | null>(null);

export { ChipGroupContext };
