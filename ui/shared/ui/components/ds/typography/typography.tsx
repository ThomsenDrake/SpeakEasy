"use client";

import type {
  ComponentPropsWithRef,
  DetailedHTMLProps,
  HTMLAttributes,
} from "react";
import { createElement, useContext } from "react";

import { TypographyContext } from "./typography-context";
import {
  type TypographyVariantProps,
  typographyVariants,
} from "./typography-variants";

type ValidTags =
  | "p"
  | "span"
  | "div"
  | "h1"
  | "h2"
  | "h3"
  | "h4"
  | "h5"
  | "h6"
  | "ul"
  | "li";
export interface TypographyProps<TTag extends ValidTags>
  extends
    DetailedHTMLProps<HTMLAttributes<HTMLDivElement>, HTMLDivElement>,
    TypographyVariantProps {
  tag?: TTag;
}
const Typography = <TTag extends ValidTags>({
  children,
  className,
  tag,
  size,
  weight,
  variant,
  ...props
}: TypographyProps<TTag>) => {
  const typographyContext = useContext(TypographyContext);
  const styles = typographyVariants({ className, size, weight, variant });

  if (tag) {
    return createElement(tag, { className: styles, ...props }, children);
  }

  const Component = typographyContext ? "span" : "p";

  return Component === "p" ? (
    <TypographyContext.Provider value={true}>
      <Component
        className={styles}
        {...(props as ComponentPropsWithRef<typeof Component>)}
      >
        {children}
      </Component>
    </TypographyContext.Provider>
  ) : (
    <Component
      className={styles}
      {...(props as ComponentPropsWithRef<typeof Component>)}
    >
      {children}
    </Component>
  );
};
Typography.displayName = "Typography";

const TypographyP = (props: TypographyProps<"p">) => (
  <Typography tag="p" {...props} />
);
TypographyP.displayName = "Typography.P";

const TypographySpan = (props: TypographyProps<"span">) => (
  <Typography tag="span" {...props} />
);
TypographySpan.displayName = "TypographySpan";

const TypographyDiv = (props: TypographyProps<"div">) => (
  <Typography tag="div" {...props} />
);
TypographyDiv.displayName = "TypographyDiv";

const TypographyH1 = (props: TypographyProps<"h1">) => (
  <Typography tag="h1" {...props} />
);
TypographyH1.displayName = "TypographyH1";

const TypographyH2 = (props: TypographyProps<"h2">) => (
  <Typography tag="h2" {...props} />
);
TypographyH2.displayName = "TypographyH2";

const TypographyH3 = (props: TypographyProps<"h3">) => (
  <Typography tag="h3" {...props} />
);
TypographyH3.displayName = "TypographyH3";

const TypographyH4 = (props: TypographyProps<"h4">) => (
  <Typography tag="h4" {...props} />
);
TypographyH4.displayName = "TypographyH4";

const TypographyH5 = (props: TypographyProps<"h5">) => (
  <Typography tag="h5" {...props} />
);
TypographyH5.displayName = "TypographyH5";

const TypographyH6 = (props: TypographyProps<"h6">) => (
  <Typography tag="h6" {...props} />
);
TypographyH6.displayName = "TypographyH6";

const TypographyUl = (props: TypographyProps<"ul">) => (
  <Typography tag="ul" {...props} />
);
TypographyUl.displayName = "TypographyUl";

const TypographyLi = (props: TypographyProps<"li">) => (
  <Typography tag="li" {...props} />
);
TypographyLi.displayName = "TypographyLi";

export {
  Typography,
  TypographyDiv,
  TypographyH1,
  TypographyH2,
  TypographyH3,
  TypographyH4,
  TypographyH5,
  TypographyH6,
  TypographyLi,
  TypographyP,
  TypographySpan,
  TypographyUl,
};
