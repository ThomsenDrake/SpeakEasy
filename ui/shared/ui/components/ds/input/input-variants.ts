import type { VariantProps } from "class-variance-authority";
import { tv } from "tailwind-variants";

export const inputVariants = tv({
  slots: {
    // wrapper is the outer element.
    // can be either the InputContainer if there is one, or else the Input.
    wrapper: [
      "w-full transition-all bg-white ring-border-default rounded-md [&>input]:rounded-none",
      "has-[input:focus]:ring-3 has-[input:focus]:shadow-input-focus",
      "[input:focus]:ring-3 [input:focus]:shadow-input-focus",
      "has-aria-invalid:border-destructive",
      "has-[input[aria-disabled='true']]:bg-input-disabled! has-[input[aria-disabled='true']]:[&,&>input]:cursor-not-allowed",
    ],
    container: "flex items-center [&>input]:grow [&>input]:px-0",
    input:
      "peer rounded-md outline-hidden placeholder-placeholder-default first:pl-3 px-3",
    leadIcon: "mx-2",
    tailIcon: "peer-aria-invalid:text-icon-destructive mx-2",
    tailButton:
      "self-stretch h-auto px-3 rounded-s-none border-e-0 border-y-0 border-s border-default ",
  },
  compoundSlots: [
    {
      slots: ["leadIcon", "tailIcon"],
      className: "size-5 p-0.5 text-icon-default-muted",
    },
  ],
  variants: {
    variant: {
      unstyled: {
        wrapper:
          "has-[input:focus]:shadow-none [input:focus]:shadow-none ring-0 ring-transparent",
      },
      default: {
        wrapper: "bg-input border border-darker",
        tailButton:
          "text-subtle hover:bg-state-secondary-hover active:bg-state-secondary-press",
      },
      soft: {
        wrapper: "bg-input-soft has-[input:focus]:bg-input",
        tailButton:
          "text-subtle hover:bg-state-soft-hover active:bg-state-soft-press",
      },
    },
    size: {
      sm: {
        input: "h-[34px]",
      },
      lg: {
        input: "h-[38px]",
      },
    },
  },
  compoundVariants: [],
  defaultVariants: {
    variant: "default",
    size: "lg",
  },
});

export type InputVariantProps = VariantProps<typeof inputVariants>;
export type InputVariantStyles = ReturnType<typeof inputVariants>;
