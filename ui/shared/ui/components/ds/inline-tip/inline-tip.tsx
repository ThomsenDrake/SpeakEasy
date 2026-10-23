"use client";

import {
  type InlineTipVariantProps,
  inlineTipVariants,
} from "./inline-tip-variants";

const stateToRole = {
  default: "status",
  information: "status",
  success: "status",
  warning: "alert",
  error: "alert",
} satisfies Record<keyof InlineTipVariantProps["state"], "status" | "alert">;

export type InlineTipProps = React.HTMLAttributes<HTMLDivElement> &
  InlineTipVariantProps;

/**
 * @see https://www.figma.com/design/FZxG4186pzrdTECGoeqYbH/%F0%9F%8E%A8-Sort-1.2---Mistral?node-id=3899-6343&t=27HuKLUuTQazkHdM-11
 */
const InlineTip = ({
  children,
  className,
  state,
  ...props
}: InlineTipProps) => {
  const styles = inlineTipVariants({ state });
  return (
    <aside
      role={stateToRole[state ?? "default"]}
      className={styles.container({ className })}
      {...props}
    >
      <div className={styles.line()} />
      <div className="min-w-0 break-words">{children}</div>
    </aside>
  );
};
InlineTip.displayName = "InlineTip";

export { InlineTip };
