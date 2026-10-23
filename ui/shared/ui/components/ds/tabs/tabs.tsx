"use client";

import * as TabsPrimitive from "@radix-ui/react-tabs";
import { motion } from "framer-motion";
import type { RefObject } from "react";
import { useContext, useId, useMemo, useState } from "react";

import { cn } from "#utils/cn.ts";

import { TabsContext } from "./tabs-context";
import { type TabsVariantProps, tabsVariants } from "./tabs-variants";

export interface TabsProps extends TabsPrimitive.TabsProps, TabsVariantProps {
  ref?: RefObject<HTMLDivElement | null>;
}

/**
 * A Tabs component that provides a flexible and accessible tabbed interface.
 *
 * @component
 * @param {TabsProps} props - Props inherited from Radix UI's TabsPrimitive and additional variant props.
 * @param {React.ReactNode} props.children - The child elements of the Tabs component.
 * @param {string} [props.variant] - The visual variant of the tabs.
 * @param {string} [props.className] - Additional CSS classes to apply.
 * @param {string} [props.defaultValue] - The default active tab value.
 * @param {(value: string) => void} [props.onValueChange] - Callback function triggered when the tab value changes.
 *
 * @example
 * <Tabs defaultValue="tab1">
 *   <TabsList>
 *     <Tab value="tab1">Tab 1</Tab>
 *     <Tab value="tab2">Tab 2</Tab>
 *   </TabsList>
 *   <TabPanels>
 *     <TabPanel value="tab1">Content 1</TabPanel>
 *     <TabPanel value="tab2">Content 2</TabPanel>
 *   </TabPanels>
 * </Tabs>
 */
const Tabs = ({
  children,
  variant,
  className,
  defaultValue,
  onValueChange,
  ...props
}: TabsProps) => {
  const [_activeTab, setActiveTab] = useState<string | undefined>(
    props.value ?? defaultValue,
  );
  const activeTab = props.value ?? _activeTab;
  const layoutId = useId();

  const handleValueChange = (value: string) => {
    setActiveTab(value);
    onValueChange?.(value);
  };

  const contextValue = useMemo(() => {
    const styles = tabsVariants({ variant });
    return { styles, activeTab, layoutId };
  }, [variant, activeTab, layoutId]);

  return (
    <TabsContext.Provider value={contextValue}>
      <TabsPrimitive.Root
        defaultValue={defaultValue}
        onValueChange={handleValueChange}
        className={contextValue.styles.root({ className })}
        {...props}
      >
        {children}
      </TabsPrimitive.Root>
    </TabsContext.Provider>
  );
};

Tabs.displayName = "Tabs";

/**
 * A component that wraps multiple Tab components.
 *
 * @component
 * @param {Object} props - Component props.
 * @param {React.ReactNode} props.children - The child elements of the TabsList.
 * @param {string} [props.className] - Additional CSS classes to apply.
 *
 * @example
 * <TabsList>
 *   <Tab value="tab1">Tab 1</Tab>
 *   <Tab value="tab2">Tab 2</Tab>
 * </TabsList>
 */
const TabsList = ({
  children,
  className,
}: {
  children: React.ReactNode;
  className?: string;
}) => {
  const context = useContext(TabsContext);
  if (!context) throw new Error("TabList must be used within a Tabs component");

  return (
    <TabsPrimitive.List className={context.styles.list({ className })}>
      {children}
    </TabsPrimitive.List>
  );
};
TabsList.displayName = "TabsList";

/**
 * A component that represents a single tab.
 *
 * @component
 * @param {Object} props - Component props.
 * @param {string} props.value - The value of the tab.
 * @param {React.ReactNode} props.children - The content of the tab.
 * @param {string} [props.className] - Additional CSS classes to apply.
 *
 * @example
 * <Tab value="tab1">Tab 1</Tab>
 */
const Tab = ({
  value,
  children,
  isDisabled,
  className,
}: {
  value: string;
  children: React.ReactNode;
  isDisabled?: boolean;
  className?: string;
}) => {
  const context = useContext(TabsContext);
  if (!context) throw new Error("Tab must be used within a Tabs component");

  const disabledStyle = isDisabled ? "opacity-50 cursor-not-allowed" : "";

  return (
    <TabsPrimitive.Trigger
      className={cn(context.styles.trigger({ className }), disabledStyle)}
      value={value}
      disabled={isDisabled}
    >
      <span className={context.styles.label()}>{children}</span>
      {context.activeTab === value && (
        <motion.span
          layoutId={`tab-${context.layoutId}`}
          transition={{ duration: 0.15 }}
          className={context.styles.indicator()}
          /**
           * https://stackoverflow.com/a/77382876
           * Avoids animating the tab underline when tablist moves on the Y axis
           */
          style={{ originY: "0px" }}
        />
      )}
    </TabsPrimitive.Trigger>
  );
};
Tab.displayName = "Tab";

/**
 * A component that acts as a wrapper for multiple TabPanel components.
 *
 * @component
 * @param {Object} props - Component props.
 * @param {React.ReactNode} props.children - The child elements of the TabPanels.
 *
 * @example
 * <TabPanels>
 *   <TabPanel value="tab1">Content 1</TabPanel>
 *   <TabPanel value="tab2">Content 2</TabPanel>
 * </TabPanels>
 */
const TabPanels = ({ children }: { children: React.ReactNode }) => {
  const context = useContext(TabsContext);

  if (!context)
    throw new Error("TabPanels must be used within a Tabs component");

  return children;
};
TabPanels.displayName = "TabPanels";

/**
 * A component that displays content corresponding to a specific tab.
 *
 * @component
 * @param {Object} props - Component props.
 * @param {string} props.value - The value of the associated tab.
 * @param {React.ReactNode} props.children - The content of the tab panel.
 * @param {string} [props.className] - Additional class names for styling.
 *
 * @example
 * <TabPanel value="tab1" className="p-4">Content 1</TabPanel>
 */
const TabPanel = ({
  value,
  children,
  className,
}: {
  value: string;
  children: React.ReactNode;
  className?: string;
}) => {
  const context = useContext(TabsContext);
  if (!context)
    throw new Error("TabPanel must be used within a Tabs component");

  return (
    <TabsPrimitive.Content
      className={context.styles.content({ className })}
      value={value}
    >
      {children}
    </TabsPrimitive.Content>
  );
};
TabPanel.displayName = "TabPanel";

export { Tab, TabPanel, TabPanels, Tabs, TabsList };
