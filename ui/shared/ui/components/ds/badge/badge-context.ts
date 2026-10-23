import { createContext } from "react";

import type { BadgeStyles } from "./badge-variants";

const BadgeContext = createContext<{ styles: BadgeStyles } | null>(null);

export { BadgeContext };
