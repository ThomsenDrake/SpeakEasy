import { createContext } from "react";

import type { BreadcrumbsStyles } from "./breadcrumbs-variants";

const BreadcrumbsContext = createContext<{
  styles: BreadcrumbsStyles;
  hasTrailingSeparator: boolean;
} | null>(null);

export { BreadcrumbsContext };
