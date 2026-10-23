import { createContext } from "react";

import type { TabsStyles } from "./tabs-variants";

const TabsContext = createContext<{
  styles: TabsStyles;
  activeTab?: string;
  layoutId?: string;
} | null>(null);

export { TabsContext };
