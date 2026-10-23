"use client";

import * as DirectionPrimitive from "@radix-ui/react-direction";

export function DirectionProvider({
  dir,
  children,
}: {
  dir: "rtl" | "ltr";
  children: React.ReactNode;
}) {
  return (
    <DirectionPrimitive.Provider dir={dir}>
      {children}
    </DirectionPrimitive.Provider>
  );
}

export const useDirection = DirectionPrimitive.useDirection;
