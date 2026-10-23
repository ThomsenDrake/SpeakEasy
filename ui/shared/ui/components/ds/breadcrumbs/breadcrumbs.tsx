"use client";

import {
  Children,
  cloneElement,
  isValidElement,
  type ReactNode,
  useContext,
  useMemo,
} from "react";
import type {
  BreadcrumbProps,
  BreadcrumbsProps as BreadcrumbsPrimitiveProps,
} from "react-aria-components";
import {
  Breadcrumb,
  Breadcrumbs as BreadcrumbsPrimitive,
} from "react-aria-components";

import { TypographySpan } from "../typography";
import { BreadcrumbsContext } from "./breadcrumbs-context";
import {
  type BreadcrumbsVariantProps,
  breadcrumbsVariants,
} from "./breadcrumbs-variants";

export interface BreadcrumbsProps
  extends
    BreadcrumbsPrimitiveProps<Record<string, never>>,
    BreadcrumbsVariantProps {
  hasTrailingSeparator?: boolean;
  children: ReactNode;
}

const Breadcrumbs = ({
  children,
  className,
  hasTrailingSeparator = false,
  ...props
}: BreadcrumbsProps) => {
  const contextValue = useMemo(
    () => ({ styles: breadcrumbsVariants(), hasTrailingSeparator }),
    [hasTrailingSeparator],
  );

  return (
    <BreadcrumbsContext.Provider value={contextValue}>
      <BreadcrumbsPrimitive
        className={contextValue.styles.root({ className })}
        {...props}
      >
        {Children.map(children, (child, index) => {
          if (!isValidElement(child)) {
            return null;
          }

          return !Array.isArray(children) || index === children.length - 1
            ? cloneElement(child, {
                isLast: true,
              } as BreadcrumbsItemProps)
            : child;
        })}
      </BreadcrumbsPrimitive>
    </BreadcrumbsContext.Provider>
  );
};
Breadcrumbs.displayName = "Breadcrumbs";

interface BreadcrumbsItemProps extends BreadcrumbProps {
  children: ReactNode;
  className?: string;
  isLast?: boolean;
}

const BreadcrumbsItem = ({
  children,
  isLast,
  className,
  ...props
}: BreadcrumbsItemProps) => {
  const breadcrumbsContext = useContext(BreadcrumbsContext);

  if (!breadcrumbsContext) {
    throw new Error("BreadcrumbsItem must be used within a Breadcrumbs");
  }

  const { styles, hasTrailingSeparator } = breadcrumbsContext;

  return (
    <Breadcrumb
      data-last-breadcrumbs-item={isLast}
      className={styles.item({ className })}
      {...props}
    >
      {children}
      {(!isLast || hasTrailingSeparator) && (
        <TypographySpan className={styles.separator({ className })}>
          /
        </TypographySpan>
      )}
    </Breadcrumb>
  );
};
BreadcrumbsItem.displayName = "BreadcrumbsItem";

export { Breadcrumbs, BreadcrumbsItem };
