"use client";

import * as TabsPrimitive from "@radix-ui/react-tabs";
import { AnimatePresence, motion } from "framer-motion";
import type { Transition } from "framer-motion";
import { Children, isValidElement, useContext, useMemo, useRef } from "react";

import { cn } from "#utils/cn.ts";

import { TabsContext } from "./tabs-context";

const DEFAULT_TRANSITION: Transition = { duration: 0.2, ease: "easeInOut" };

interface AnimatedTabPanelProps {
  value: string;
  children: React.ReactNode;
  className?: string;
}

const AnimatedTabPanel = ({ children, className }: AnimatedTabPanelProps) => {
  return <div className={className}>{children}</div>;
};
AnimatedTabPanel.displayName = "AnimatedTabPanel";

interface AnimatedTabPanelsProps {
  children: React.ReactNode;
  className?: string;
  wrapperClassName?: string;
  transition?: Transition;
}

const AnimatedTabPanels = ({
  children,
  className,
  wrapperClassName,
  transition = DEFAULT_TRANSITION,
}: AnimatedTabPanelsProps) => {
  const context = useContext(TabsContext);
  if (!context)
    throw new Error("AnimatedTabPanels must be used within a Tabs component");

  const { activeTab } = context;
  const previousTabRef = useRef(activeTab);
  const isForwardRef = useRef(true);

  const { tabOrder, activeChild } = useMemo(() => {
    const order: Record<string, number> = {};
    let index = 0;
    let match: React.ReactElement<AnimatedTabPanelProps> | null = null;

    Children.forEach(children, (child) => {
      if (isValidElement<AnimatedTabPanelProps>(child) && child.props.value) {
        order[child.props.value] = index++;
        if (child.props.value === activeTab) {
          match = child;
        }
      }
    });

    return {
      tabOrder: order,
      activeChild: match as React.ReactElement<AnimatedTabPanelProps> | null,
    };
  }, [children, activeTab]);

  if (activeTab !== previousTabRef.current) {
    isForwardRef.current =
      (tabOrder[activeTab ?? ""] ?? 0) >
      (tabOrder[previousTabRef.current ?? ""] ?? 0);
    previousTabRef.current = activeTab;
  }

  const variants = useMemo(
    () => ({
      enter: (forward: boolean) => ({
        x: forward ? "100%" : "-100%",
        opacity: 0,
      }),
      center: { x: 0, opacity: 1 },
      exit: (forward: boolean) => ({
        x: forward ? "-100%" : "100%",
        opacity: 0,
      }),
    }),
    [],
  );

  return (
    <div className={cn("min-h-0 flex-1 overflow-hidden", wrapperClassName)}>
      <AnimatePresence
        mode="wait"
        initial={false}
        custom={isForwardRef.current}
      >
        {activeChild && activeTab && (
          <TabsPrimitive.Content value={activeTab} forceMount asChild>
            <motion.div
              key={activeTab}
              custom={isForwardRef.current}
              variants={variants}
              initial="enter"
              animate="center"
              exit="exit"
              transition={transition}
              className={cn(
                context.styles.content({ className }),
                "flex min-h-0 flex-1 flex-col",
                activeChild.props.className,
              )}
            >
              {activeChild.props.children}
            </motion.div>
          </TabsPrimitive.Content>
        )}
      </AnimatePresence>
    </div>
  );
};
AnimatedTabPanels.displayName = "AnimatedTabPanels";

export { AnimatedTabPanel, AnimatedTabPanels };
export type { AnimatedTabPanelProps, AnimatedTabPanelsProps };
