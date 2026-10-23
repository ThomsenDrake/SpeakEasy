"use client";

import { LockKeyhole } from "lucide-react";
import type { ReactNode } from "react";
import React from "react";

import { Badge } from "#components/ds/badge/badge.tsx";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "#components/ds/tooltip/tooltip.tsx";

import * as Sidebar from "./index";
import { useSidebar } from "./sidebar";

type LinkComponentType = React.FC<{
  children: ReactNode;
  href: string;
  target?: string;
  rel?: string;
}>;

export interface MenuLinkProps {
  href?: string;
  label: string;
  icon?: ReactNode;
  locked?: { condition: boolean; tooltip: string };
  isActive?: boolean;
  LinkComponent?: LinkComponentType;
  badge?: string;
  target?: string;
  rel?: string;
}

export const MenuLink = ({
  href,
  label,
  icon,
  locked,
  isActive,
  // to avoid next dependency in ui package
  LinkComponent,
  badge,
  target,
  rel,
}: MenuLinkProps) => {
  const { isMobile, setOpenMobile } = useSidebar({ required: true });

  const closeMobile = () => {
    if (isMobile) {
      setOpenMobile(false);
    }
  };

  return (
    <Sidebar.MenuItem key={href}>
      {locked?.condition === true ? (
        <Tooltip>
          <TooltipTrigger asChild>
            <Sidebar.MenuButton className="flex items-center gap-2 py-2 opacity-20">
              {icon}
              <span>{label}</span>
              <div className="grow" />
              <LockKeyhole />
            </Sidebar.MenuButton>
          </TooltipTrigger>
          <TooltipContent side="right">
            <p>{locked.tooltip}</p>
          </TooltipContent>
        </Tooltip>
      ) : (
        <Sidebar.MenuButton
          className="flex items-center gap-2 py-2"
          asChild
          isActive={isActive}
          onClick={closeMobile}
        >
          {LinkComponent && href ? (
            <LinkComponent href={href} target={target} rel={rel}>
              {icon}
              <span>{label}</span>
              {badge && (
                <Badge size="sm" className="ms-auto">
                  {badge}
                </Badge>
              )}
            </LinkComponent>
          ) : (
            <div>
              {icon}
              <span>{label}</span>
              {badge && <Badge>{badge}</Badge>}
            </div>
          )}
        </Sidebar.MenuButton>
      )}
    </Sidebar.MenuItem>
  );
};
