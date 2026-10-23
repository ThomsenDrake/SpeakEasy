import { createContext } from "react";

import type { ChipStyles } from "./chip-variants";

const ChipContext = createContext<{ styles: ChipStyles } | null>(null);

export { ChipContext };
