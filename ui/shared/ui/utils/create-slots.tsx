"use client";
import {
  Children,
  isValidElement,
  type JSXElementConstructor,
  type ReactElement,
  type ReactNode,
} from "react";

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type TComponent = JSXElementConstructor<any> & {
  displayName: string;
};

/**
 * Triages children into slots based on a slots definition.
 * Unrecognized children are placed in the `main` slot.
 * Allows for components to react to the presence of specific children.
 */
export const createSlots = <TSlot extends string>(
  children: ReactNode,
  slotsDefinition: Record<TSlot, TComponent>,
) => {
  const main: ReactNode[] = [];

  const map = new Map(
    Object.entries(slotsDefinition).map(
      ([slotName, SlotComponentDefinition]) => [
        SlotComponentDefinition,
        slotName,
      ],
    ),
  );

  const slots = {} as Record<TSlot, ReactNode>;

  Children.forEach(children, (child) => {
    if (!isValidElement(child)) {
      main.push(child);
      return;
    }

    let slotName = map.get(child.type);

    // Handles HMR (where the identity of the component changes between renders and the identity check in `map.get()` doesn't work anymore)
    if (!slotName && process.env.NODE_ENV === "development") {
      slotName = Object.entries(slotsDefinition).find(
        ([_, value]) =>
          isValidElement(child) &&
          (value as TComponent).displayName ===
            (child.type as TComponent).displayName,
      )?.[0] as TSlot | undefined;
    }

    if (slotName) {
      slots[slotName as TSlot] = child;
    } else {
      main.push(child);
    }
  });

  return {
    main,
    ...(slots as Record<TSlot, ReactElement | undefined>),
  };
};
