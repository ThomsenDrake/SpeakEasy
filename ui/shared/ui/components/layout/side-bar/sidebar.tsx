"use client";

import { Slot } from "@radix-ui/react-slot";
import type { VariantProps } from "class-variance-authority";
import { cva } from "class-variance-authority";
import { AnimatePresence, motion } from "framer-motion";
import { PanelLeft } from "lucide-react";
import * as React from "react";
import { useEventCallback } from "usehooks-ts";

import { useHotkey } from "../../../_stubs/use-hotkey";

import { Button } from "#components/ds/button/button.tsx";
import { Divider } from "#components/ds/divider/divider.tsx";
import type { TooltipProps } from "#components/ds/tooltip/tooltip.tsx";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "#components/ds/tooltip/tooltip.tsx";
import {
  Content as SheetContent,
  Sheet,
  Title as SheetTitle,
} from "#components/ui/sheet.tsx";
import { Skeleton } from "#components/ui/skeleton.tsx";
import { useDeviceType } from "#hooks/use-device-width.tsx";
import { cn } from "#utils/cn.ts";

const SIDEBAR_COOKIE_NAME = "sidebar:state";
const SIDEBAR_COOKIE_MAX_AGE = 60 * 60 * 24 * 7;
const SIDEBAR_WIDTH = "16.25rem";
const SIDEBAR_WIDTH_MOBILE = "18rem";
const SIDEBAR_WIDTH_ICON = "3rem";

type OptionsType = {
  persistent?: boolean | undefined;
};

interface SidebarContext {
  state: "expanded" | "collapsed";
  open: boolean;
  setOpen: (
    open: boolean,
    options?: {
      /**
       * When set to false, it won't save the state into cookies
       */
      persistent?: boolean;
    },
  ) => void;
  openMobile: boolean;
  setOpenMobile: (open: boolean) => void;
  isMobile: boolean;
  toggleSidebar: () => void;
}

const SidebarContext = React.createContext<SidebarContext | null>(null);

function useSidebar<TRequired extends boolean>({
  required,
}: {
  required: TRequired;
}) {
  const context = React.useContext(SidebarContext);

  if (required && !context) {
    throw new Error("useSidebar must be used within a SidebarProvider.");
  }

  return context as TRequired extends true
    ? SidebarContext
    : SidebarContext | null;
}

const Provider = ({
  defaultOpen = true,
  open: openProp,
  onOpenChange: setOpenProp,
  className,
  style,
  children,
  ref,
  ...props
}: React.ComponentProps<"div"> & {
  defaultOpen?: boolean;
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
}) => {
  const isMobile = useDeviceType() === "mobile";
  const [openMobile, setOpenMobile] = React.useState(false);

  // This is the internal state of the sidebar.
  // We use openProp and setOpenProp for control from outside the component.
  const [_open, _setOpen] = React.useState(defaultOpen);
  const open = openProp ?? _open;

  const setOpen = useEventCallback(
    (
      value: boolean | ((value: boolean) => boolean),
      options: OptionsType = { persistent: true },
    ) => {
      const openState = typeof value === "function" ? value(open) : value;
      if (setOpenProp) {
        setOpenProp(openState);
      } else {
        _setOpen(openState);
      }

      // This sets the cookie to keep the sidebar state. If persistent is set false the cookie won't change
      if (options.persistent !== false) {
        document.cookie = `${SIDEBAR_COOKIE_NAME}=${openState}; path=/; max-age=${SIDEBAR_COOKIE_MAX_AGE}`;
      }
    },
  );

  // Helper to toggle the sidebar.
  const toggleSidebar = React.useCallback(() => {
    return isMobile ? setOpenMobile((open) => !open) : setOpen((open) => !open);
  }, [isMobile, setOpen, setOpenMobile]);

  useHotkey("mod+b", () => {
    toggleSidebar();
  });

  // We add a state so that we can do data-state="expanded" or "collapsed".
  // This makes it easier to style the sidebar with Tailwind classes.
  const state = open ? "expanded" : "collapsed";

  const contextValue = React.useMemo<SidebarContext>(
    () => ({
      state,
      open,
      setOpen,
      isMobile,
      openMobile,
      setOpenMobile,
      toggleSidebar,
    }),
    [state, open, setOpen, isMobile, openMobile, setOpenMobile, toggleSidebar],
  );

  return (
    <SidebarContext.Provider value={contextValue}>
      <div
        style={
          {
            "--sidebar-width": SIDEBAR_WIDTH,
            "--sidebar-width-icon": SIDEBAR_WIDTH_ICON,
            ...style,
          } as React.CSSProperties
        }
        className={cn(
          "group/sidebar-wrapper has-data-[variant=inset]:bg-sidebar-subtle flex min-h-svh w-full",
          className,
        )}
        ref={ref}
        {...props}
      >
        {children}
      </div>
    </SidebarContext.Provider>
  );
};

Provider.displayName = "SidebarProvider";

const Sidebar = ({
  side = "left",
  variant = "sidebar",
  collapsible = "offcanvas",
  className,
  children,
  ref,
  ...props
}: React.ComponentProps<"div"> & {
  side?: "left" | "right";
  variant?: "sidebar" | "floating" | "inset";
  collapsible?: "offcanvas" | "icon" | "none";
}) => {
  const { isMobile, state, openMobile, setOpenMobile, toggleSidebar } =
    useSidebar({
      required: true,
    });

  if (collapsible === "none") {
    return (
      <div
        className={cn(
          "bg-sidebar-subtle text-subtle flex h-full w-(--sidebar-width) flex-col",
          className,
        )}
        ref={ref}
        {...props}
      >
        {children}
      </div>
    );
  }

  if (isMobile) {
    return (
      <Sheet open={openMobile} onOpenChange={setOpenMobile} {...props}>
        <SheetTitle className="sr-only">Sidebar</SheetTitle>
        <SheetContent
          data-sidebar="sidebar"
          data-mobile="true"
          className="bg-sidebar-subtle text-subtle w-(--sidebar-width) p-0 [&>button]:hidden"
          style={
            {
              "--sidebar-width": SIDEBAR_WIDTH_MOBILE,
            } as React.CSSProperties
          }
          side={side}
        >
          <div className="flex h-full w-full flex-col">{children}</div>
        </SheetContent>
      </Sheet>
    );
  }

  return (
    <div
      ref={ref}
      className="text-subtle group peer hidden md:block"
      data-state={state}
      data-collapsible={state === "collapsed" ? collapsible : ""}
      data-variant={variant}
      data-side={side}
    >
      {/* This is what handles the sidebar gap on desktop */}
      <div
        className={cn(
          "relative h-svh w-(--sidebar-width) bg-transparent transition-[width] duration-200 ease-in-out",
          "group-data-[collapsible=offcanvas]:w-0",
          "group-data-[side=right]:rotate-180",
          variant === "floating" || variant === "inset"
            ? "group-data-[collapsible=icon]:w-[calc(var(--sidebar-width-icon)+(--spacing(4)))]"
            : "group-data-[collapsible=icon]:w-(--sidebar-width-icon)",
        )}
      />
      <div
        className={cn(
          "fixed inset-y-0 z-10 hidden h-svh w-(--sidebar-width) transition-[inset-inline-start,inset-inline-end,width] duration-200 ease-in-out md:flex",
          side === "left"
            ? "start-0 group-data-[collapsible=offcanvas]:start-[calc(var(--sidebar-width)*-1)]"
            : "end-0 group-data-[collapsible=offcanvas]:end-[calc(var(--sidebar-width)*-1)]",
          // Adjust the padding for floating and inset variants.
          variant === "floating" || variant === "inset"
            ? "p-2 group-data-[collapsible=icon]:w-[calc(var(--sidebar-width-icon)+(--spacing(4))+2px)]"
            : "group-data-[collapsible=icon]:w-(--sidebar-width-icon) group-data-[side=left]:border-e group-data-[side=right]:border-s",
          className,
        )}
        {...props}
      >
        <div
          data-sidebar="sidebar"
          className={cn(
            "bg-sidebar-subtle group-data-[variant=floating]:border-sidebar-border flex h-full w-full flex-col group-data-[variant=floating]:rounded-lg group-data-[variant=floating]:border group-data-[variant=floating]:shadow-2xs",
            "group-data-[collapsible=icon]:bg-subtle hover:group-data-[collapsible=icon]:bg-muted group-data-[collapsible=icon]:border-0 group-data-[collapsible=icon]:shadow-none hover:group-data-[collapsible=icon]:cursor-pointer",
          )}
          onClick={() => {
            if (state === "collapsed" && collapsible === "icon") {
              toggleSidebar();
            }
          }}
        >
          <AnimatePresence mode="wait" initial={false}>
            {state === "collapsed" && collapsible === "icon" ? (
              <motion.div
                key="icon"
                initial={{ opacity: 0 }}
                animate={{ opacity: 1 }}
                exit={{ opacity: 0 }}
                transition={{ duration: 0.1, ease: "easeInOut" }}
                className="desktop-mac:pt-13 flex items-center justify-center pt-4"
              >
                <Button
                  variant="ghost"
                  size="sm"
                  mode="icon-only"
                  icon={PanelLeft}
                  onClick={toggleSidebar}
                  className="text-icon-default-muted"
                />
              </motion.div>
            ) : (
              <motion.div
                key="content"
                initial={{
                  opacity: 0,
                }}
                animate={{
                  opacity: 1,
                  transition: { duration: 0.2, ease: "easeInOut" },
                }}
                exit={{
                  opacity: 0,
                  transition: { duration: 0.125, ease: "easeInOut" },
                }}
                className="flex h-full w-full flex-col"
              >
                {children}
              </motion.div>
            )}
          </AnimatePresence>
        </div>
      </div>
    </div>
  );
};
Sidebar.displayName = "Sidebar";

const Rail = ({ className, ref, ...props }: React.ComponentProps<"button">) => {
  const { toggleSidebar } = useSidebar({ required: true });

  return (
    <button
      ref={ref}
      data-sidebar="rail"
      aria-label="Toggle Sidebar"
      tabIndex={-1}
      onClick={toggleSidebar}
      title="Toggle Sidebar"
      className={cn(
        "hover:after:bg-muted absolute inset-y-0 z-20 hidden w-4 -translate-x-1/2 transition-all ease-in-out group-data-[side=left]:-right-4 group-data-[side=right]:left-0 after:absolute after:inset-y-0 after:left-1/2 after:w-[2px] sm:flex",
        "in-data-[side=left]:cursor-w-resize in-data-[side=right]:cursor-e-resize",
        "[[data-side=left][data-state=collapsed]_&]:cursor-e-resize [[data-side=right][data-state=collapsed]_&]:cursor-w-resize",
        "hover:group-data-[collapsible=offcanvas]:bg-sidebar-subtle group-data-[collapsible=offcanvas]:translate-x-0 group-data-[collapsible=offcanvas]:after:left-full",
        "[[data-side=left][data-collapsible=offcanvas]_&]:-right-2",
        "[[data-side=right][data-collapsible=offcanvas]_&]:-left-2",
        className,
      )}
      {...props}
    />
  );
};
Rail.displayName = "SidebarRail";

const Inset = ({ className, ref, ...props }: React.ComponentProps<"main">) => (
  <main
    ref={ref}
    className={cn(
      "bg-sidebar-subtle relative flex min-h-svh flex-1 flex-col overflow-hidden",
      "h-dvh max-h-dvh min-h-dvh peer-data-[variant=inset]:min-h-[calc(100svh-(--spacing(4)))] md:p-0 md:peer-data-[state=expanded]:pl-0",
      className,
    )}
    {...props}
  />
);
Inset.displayName = "SidebarInset";

const Header = ({ className, ref, ...props }: React.ComponentProps<"div">) => (
  <div
    ref={ref}
    data-sidebar="header"
    className={cn("flex flex-col gap-2 p-2", className)}
    {...props}
  />
);
Header.displayName = "SidebarHeader";

const Footer = ({ className, ref, ...props }: React.ComponentProps<"div">) => (
  <div
    ref={ref}
    data-sidebar="footer"
    className={cn("flex flex-col gap-2 p-2", className)}
    {...props}
  />
);
Footer.displayName = "SidebarFooter";

const Separator = ({
  className,
  ...props
}: React.ComponentProps<typeof Divider>) => (
  <Divider
    data-sidebar="separator"
    className={cn("bg-muted mx-2 w-auto", className)}
    {...props}
  />
);
Separator.displayName = "SidebarSeparator";

const Content = ({ className, ref, ...props }: React.ComponentProps<"div">) => (
  <div
    ref={ref}
    data-sidebar="content"
    className={cn(
      "flex min-h-0 flex-1 flex-col gap-2 overflow-auto group-data-[collapsible=icon]:overflow-hidden",
      className,
    )}
    {...props}
  />
);
Content.displayName = "SidebarContent";

const Group = ({
  className,
  ref,
  ...props
}: React.ComponentProps<"div"> & { ref?: React.Ref<HTMLDivElement> }) => {
  return (
    <div
      ref={ref}
      data-sidebar="group"
      className={cn("relative flex w-full min-w-0 flex-col p-2", className)}
      {...props}
    />
  );
};
Group.displayName = "SidebarGroup";

const GroupLabel = ({
  className,
  asChild = false,
  ref,
  ...props
}: React.ComponentProps<"div"> & { asChild?: boolean }) => {
  const Comp = asChild ? Slot : "div";

  return (
    <Comp
      ref={ref}
      data-sidebar="group-label"
      className={cn(
        "ring-default text-default flex h-8 shrink-0 items-center rounded-md px-2 text-xs font-medium outline-hidden transition-[margin,opa] duration-200 ease-in-out focus-visible:ring-2 [&>svg]:size-4 [&>svg]:shrink-0",
        className,
      )}
      {...props}
    />
  );
};
GroupLabel.displayName = "SidebarGroupLabel";

const GroupAction = ({
  className,
  asChild = false,
  ref,
  ...props
}: React.ComponentProps<"button"> & {
  asChild?: boolean;
}) => {
  const Comp = asChild ? Slot : "button";

  return (
    <Comp
      ref={ref}
      data-sidebar="group-action"
      className={cn(
        "text-subtle ring-default hover:bg-muted hover:text-default absolute top-3.5 right-3 flex aspect-square w-5 items-center justify-center rounded-md p-0 outline-hidden transition-transform focus-visible:ring-2 [&>svg]:size-4 [&>svg]:shrink-0",
        "after:absolute after:-inset-2 md:after:hidden",
        className,
      )}
      {...props}
    />
  );
};
GroupAction.displayName = "SidebarGroupAction";

const GroupContent = ({
  className,
  ref,
  ...props
}: React.ComponentProps<"div">) => (
  <div
    ref={ref}
    data-sidebar="group-content"
    className={cn("w-full text-sm", className)}
    {...props}
  />
);
GroupContent.displayName = "SidebarGroupContent";

const Menu = ({ className, ref, ...props }: React.ComponentProps<"ul">) => (
  <ul
    ref={ref}
    data-sidebar="menu"
    className={cn("flex w-full min-w-0 flex-col gap-1", className)}
    {...props}
  />
);
Menu.displayName = "SidebarMenu";

const MenuItem = ({ className, ref, ...props }: React.ComponentProps<"li">) => (
  <li
    ref={ref}
    data-sidebar="menu-item"
    className={cn("group/menu-item relative", className)}
    {...props}
  />
);
MenuItem.displayName = "SidebarMenuItem";

const MenuMainButton = ({
  asChild = false,
  isActive = false,
  className,
  role,
  ref,
  ...props
}: React.ComponentProps<"div"> & {
  as?: keyof Pick<React.JSX.IntrinsicElements, "div" | "button">;
  asChild?: boolean;
  isActive?: boolean;
  tooltip?: string | TooltipProps;
}) => {
  const Comp = asChild ? Slot : "div";

  return (
    <Comp
      role={asChild ? undefined : role || "button"}
      ref={ref}
      data-sidebar="menu-main-button"
      data-active={isActive}
      className={cn(
        "ring-default text-default flex h-10 w-full shrink-0 items-center border-s-2 border-e-2 border-transparent px-5.5 text-sm font-[450] outline-hidden transition-[margin,opa] duration-200 ease-in-out focus-visible:ring-2 [&>svg]:size-4 [&>svg]:shrink-0",
        "hover:bg-state-soft active:bg-badge-orangebright data-[active=true]:bg-badge-orangebright data-[active=true]:font-medium",
        "hover:border-s-brand-500 active:border-s-brand-500 data-[active=true]:border-s-brand-500 transition-colors",
        className,
      )}
      {...props}
    />
  );
};
MenuMainButton.displayName = "SidebarMenuMainButton";

const MenuMainButtonOption = ({
  asChild = false,
  isActive = false,
  children,
  className,
  ref,
  ...props
}: React.ComponentProps<"div"> & {
  asChild?: boolean;
  isActive?: boolean;
}) => {
  const Comp = asChild ? Slot : "div";

  return (
    <div className="relative">
      <Comp
        ref={ref}
        data-sidebar="menu-main-button-option"
        data-active={isActive}
        className={cn(
          "text-subtle flex min-h-6 w-full flex-row items-center justify-between rounded-lg p-2 pl-[2.5rem] text-sm transition-colors sm:min-h-6",
          "hover:bg-state-soft",
          "data-[active=true]:bg-state-soft",
          className,
        )}
        {...props}
      >
        {children}
      </Comp>
      <div className="absolute top-0 bottom-0 left-4 flex w-px items-center">
        <AnimatePresence>
          {isActive && (
            <motion.div
              className="bg-basic-orange-accent h-[60%] w-full"
              initial={{ opacity: 0, y: -10 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: 10 }}
            />
          )}
        </AnimatePresence>
        <div className="bg-basic-gray-alpha-4 absolute top-0 bottom-0 w-full" />
      </div>
    </div>
  );
};
MenuMainButtonOption.displayName = "SidebarMenuMainButtonOption";

const sidebarMenuButtonVariants = cva(
  "peer/menu-button ring-default hover:bg-state-soft hover:text-default active:bg-state-soft active:text-default data-[active=true]:bg-state-soft data-[active=true]:text-default data-[state=open]:hover:bg-state-soft data-[state=open]:hover:text-default flex w-full items-center gap-2 overflow-hidden rounded-md p-2 text-left text-sm outline-hidden transition-[width,height,padding] transition-colors group-has-data-[sidebar=menu-action]/menu-item:pr-8 focus-visible:ring-2 disabled:pointer-events-none disabled:opacity-50 aria-disabled:pointer-events-none aria-disabled:opacity-50 data-[active=true]:font-medium [&>span:last-child]:truncate [&>svg]:size-4 [&>svg]:shrink-0",
  {
    variants: {
      variant: {
        default: "hover:bg-state-soft hover:text-default",
        outline:
          "bg-default hover:bg-state-soft hover:text-default shadow-[0_0_0_1px_hsl(var(--sidebar-border))] hover:shadow-[0_0_0_1px_hsl(var(--sidebar-accent))]",
      },
      size: {
        default: "h-8 text-sm",
        sm: "h-7 text-xs",
        lg: "h-12 text-sm",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  },
);

const MenuButton = ({
  asChild = false,
  isActive = false,
  variant = "default",
  size = "default",
  tooltip,
  className,
  ref,
  ...props
}: React.ComponentProps<"button"> & {
  asChild?: boolean;
  isActive?: boolean;
  tooltip?: string | TooltipProps;
} & VariantProps<typeof sidebarMenuButtonVariants>) => {
  const Comp = asChild ? Slot : "button";

  const { isMobile, state } = useSidebar({ required: true });

  const button = (
    <Comp
      ref={ref}
      data-sidebar="menu-button"
      data-size={size}
      data-active={isActive}
      className={cn(sidebarMenuButtonVariants({ variant, size }), className)}
      {...props}
    />
  );

  if (!tooltip) {
    return button;
  }

  if (typeof tooltip === "string") {
    tooltip = {
      children: tooltip,
    };
  }

  return (
    <Tooltip>
      <TooltipTrigger asChild>{button}</TooltipTrigger>
      <TooltipContent
        side="right"
        align="center"
        hidden={state !== "collapsed" || isMobile}
        {...tooltip}
      />
    </Tooltip>
  );
};
MenuButton.displayName = "SidebarMenuButton";

const MenuCommand = ({
  asChild = false,
  className,
  ref,
  ...props
}: React.ComponentProps<"button"> & {
  asChild?: boolean;
}) => {
  const Comp = asChild ? Slot : "button";

  return (
    <Comp
      ref={ref}
      data-sidebar="menu-command"
      className={cn(
        "ring-default text-hint hover:text-muted flex h-10 w-full shrink-0 items-center border-s-1 border-e-1 border-transparent px-5.5 text-sm font-medium outline-hidden transition-all duration-150 ease-in-out focus-visible:ring-2 [&>svg]:size-4 [&>svg]:shrink-0",
        "hover:bg-state-soft",
        "hover:border-s-brand-500",
        className,
      )}
      {...props}
    />
  );
};
MenuMainButton.displayName = "SidebarMenuMainButton";

const MenuAction = ({
  className,
  asChild = false,
  showOnHover = false,
  ref,
  ...props
}: React.ComponentProps<"button"> & {
  asChild?: boolean;
  showOnHover?: boolean;
}) => {
  const Comp = asChild ? Slot : "button";

  return (
    <Comp
      ref={ref}
      data-sidebar="menu-action"
      className={cn(
        "text-subtle ring-default hover:bg-muted hover:text-default peer-hover/menu-button:text-default absolute top-1.5 right-1 flex aspect-square w-5 items-center justify-center rounded-md p-0 outline-hidden transition-transform focus-visible:ring-2 [&>svg]:size-4 [&>svg]:shrink-0",
        "after:absolute after:-inset-2 md:after:hidden",
        "peer-data-[size=sm]/menu-button:top-1",
        "peer-data-[size=default]/menu-button:top-1.5",
        "peer-data-[size=lg]/menu-button:top-2.5",
        showOnHover &&
          "peer-data-[active=true]/menu-button:text-default group-focus-within/menu-item:opacity-100 group-hover/menu-item:opacity-100 data-[state=open]:opacity-100 md:opacity-0",
        className,
      )}
      {...props}
    />
  );
};
MenuAction.displayName = "SidebarMenuAction";

const MenuBadge = ({
  className,
  ref,
  ...props
}: React.ComponentProps<"div">) => (
  <div
    ref={ref}
    data-sidebar="menu-badge"
    className={cn(
      "text-subtle pointer-events-none absolute right-1 flex h-5 min-w-5 items-center justify-center rounded-md px-1 text-xs font-medium tabular-nums select-none",
      "peer-hover/menu-button:text-default peer-data-[active=true]/menu-button:text-default",
      "peer-data-[size=sm]/menu-button:top-1",
      "peer-data-[size=default]/menu-button:top-1.5",
      "peer-data-[size=lg]/menu-button:top-2.5",
      className,
    )}
    {...props}
  />
);
MenuBadge.displayName = "SidebarMenuBadge";

const MenuSkeleton = ({
  className,
  showIcon = false,
  ref,
  ...props
}: React.ComponentProps<"div"> & {
  showIcon?: boolean;
}) => {
  // Random width between 50 to 90%.
  const width = React.useMemo(() => {
    return `${Math.floor(Math.random() * 40) + 50}%`;
  }, []);

  return (
    <div
      ref={ref}
      data-sidebar="menu-skeleton"
      className={cn("flex h-8 items-center gap-2 rounded-md px-2", className)}
      {...props}
    >
      {showIcon && (
        <Skeleton
          className="size-4 rounded-md"
          data-sidebar="menu-skeleton-icon"
        />
      )}
      <Skeleton
        className="h-4 max-w-(--skeleton-width) flex-1"
        data-sidebar="menu-skeleton-text"
        style={
          {
            "--skeleton-width": width,
          } as React.CSSProperties
        }
      />
    </div>
  );
};
MenuSkeleton.displayName = "SidebarMenuSkeleton";

const MenuSub = ({ className, ref, ...props }: React.ComponentProps<"ul">) => (
  <ul
    ref={ref}
    data-sidebar="menu-sub"
    className={cn("flex min-w-0 flex-col gap-1 py-0.5", className)}
    {...props}
  />
);
MenuSub.displayName = "SidebarMenuSub";

const MenuSubItem = ({ ref, ...props }: React.ComponentProps<"li">) => (
  <li ref={ref} {...props} />
);
MenuSubItem.displayName = "SidebarMenuSubItem";

const MenuSubButton = ({
  asChild = false,
  size = "md",
  isActive,
  className,
  ref,
  ...props
}: React.ComponentProps<"a"> & {
  asChild?: boolean;
  size?: "sm" | "md";
  isActive?: boolean;
}) => {
  const Comp = asChild ? Slot : "a";

  return (
    <Comp
      ref={ref}
      data-sidebar="menu-sub-button"
      data-size={size}
      data-active={isActive}
      className={cn(
        "text-subtle ring-default hover:bg-muted hover:text-default active:bg-muted active:text-default [&>svg]:text-default flex h-7 min-w-0 -translate-x-px items-center gap-2 overflow-hidden rounded-md px-2 outline-hidden focus-visible:ring-2 disabled:pointer-events-none disabled:opacity-50 aria-disabled:pointer-events-none aria-disabled:opacity-50 [&>span:last-child]:truncate [&>svg]:size-4 [&>svg]:shrink-0",
        "data-[active=true]:bg-muted data-[active=true]:text-default",
        size === "sm" && "text-xs",
        size === "md" && "text-sm",
        className,
      )}
      {...props}
    />
  );
};
MenuSubButton.displayName = "SidebarMenuSubButton";

export {
  Content,
  Footer,
  Group,
  GroupAction,
  GroupContent,
  GroupLabel,
  Header,
  Inset,
  Menu,
  MenuAction,
  MenuBadge,
  MenuButton,
  MenuCommand,
  MenuItem,
  MenuMainButton,
  MenuMainButtonOption,
  MenuSkeleton,
  MenuSub,
  MenuSubButton,
  MenuSubItem,
  Provider,
  Rail,
  Separator,
  Sidebar,
  useSidebar,
};
