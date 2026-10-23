"use client";

import type { HTMLAttributes, ReactElement } from "react";

import { createSlots } from "#utils/create-slots.tsx";

import { type DividerVariantProps, dividerVariants } from "./divider-variants";

export type DividerProps = Omit<HTMLAttributes<HTMLHRElement>, "children"> &
  DividerVariantProps & {
    children?: ReactElement;
  };

/**
 * @see https://www.figma.com/design/FZxG4186pzrdTECGoeqYbH/%F0%9F%8E%A8-Sort-1.2---Mistral?node-id=2044-1296&t=AZLiG3NnjvogAcgy-4
 */
function Divider({ children, className, variant, ...props }: DividerProps) {
  const slots = createSlots(children, {
    label: DividerLabel,
  });
  const styles = dividerVariants({ variant });

  if (slots.label) {
    return (
      <div className={styles.container({ className })}>
        <hr className={styles.base()} {...props} />
        {slots.label}
        <hr className={styles.base()} {...props} />
      </div>
    );
  }

  return <hr className={styles.base({ className })} {...props} />;
}
Divider.displayName = "Divider";

type DividerLabelProps = HTMLAttributes<HTMLDivElement> & {
  position?: "start" | "end" | "center";
};

function DividerLabel({
  className,
  position = "center",
  ...props
}: DividerLabelProps) {
  return (
    <span
      data-position={position}
      className={dividerVariants().label({ className })}
      {...props}
    />
  );
}
DividerLabel.displayName = "DividerLabel";

export { Divider, DividerLabel };
