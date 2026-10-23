"use client";

import type { LucideIcon } from "lucide-react";
import type { HTMLAttributes } from "react";
import { createElement, useContext, useMemo, useRef } from "react";

import { useInputAutofocusHandler } from "#hooks/use-input-autofocus-handler.ts";

import { cn } from "../../../utils/cn";
import { createSlots } from "../../../utils/create-slots";
import { Button, type ButtonProps } from "../button/button";
import { FieldContext } from "../form/form-field-context";
import { InputContainerContext } from "./input-container-context";
import { type InputVariantProps, inputVariants } from "./input-variants";

export interface InputProps
  extends
    Omit<React.InputHTMLAttributes<HTMLInputElement>, "size" | "disabled">,
    Pick<InputVariantProps, "size" | "variant"> {
  ref?: React.Ref<HTMLInputElement | null>;
  isDisabled?: boolean;
  isInvalid?: boolean;
}

/**
 * Input component for collecting user text input with various styling options and configurations.
 *
 * @see https://www.figma.com/design/FZxG4186pzrdTECGoeqYbH/%F0%9F%8E%A8-Sort-1.2---Mistral?node-id=4041-80035&t=AZLiG3NnjvogAcgy-11
 *
 * @component
 * @param {string} [props.className] - Additional CSS classes
 * @param {'default' | 'soft'} [props.variant='default'] - Visual style variant of the input
 * @param {'sm' | 'lg'} [props.size='lg'] - Size of the input
 * @param {boolean} [props.isDisabled=false] - Whether the input is disabled
 * @param {boolean} [props.isInvalid=false] - Whether the input has an invalid state
 * @param {string} [props.placeholder] - Placeholder text
 *
 * @example
 * // Basic input
 * <Input placeholder="Enter your name" />
 *
 * @example
 * // Input with variants and states
 * <Input
 *   variant="soft"
 *   size="sm"
 *   isInvalid={true}
 *   placeholder="Email address"
 * />
 */
const Input = ({
  className,
  variant,
  size,
  isDisabled,
  isInvalid,
  ...props
}: InputProps) => {
  // Dependency injection
  const fieldContext = useContext(FieldContext);
  const inputContainerContext = useContext(InputContainerContext);

  // Styles management
  const variants = inputVariants({ size, variant });
  const styles = inputContainerContext
    ? inputContainerContext.styles.input()
    : [variants.wrapper(), variants.input()];

  // Autofocus management
  const internalRef = useRef<HTMLInputElement | null>(null);
  useInputAutofocusHandler(internalRef, props.autoFocus);

  return (
    <input
      ref={(node) => {
        internalRef.current = node;
        if (typeof props.ref === "function") {
          props.ref(node);
        } else if (props.ref) {
          props.ref.current = node;
        }
      }}
      name={fieldContext?.name}
      id={fieldContext?.id || props.id}
      aria-invalid={isInvalid || fieldContext?.isInvalid}
      aria-disabled={isDisabled}
      disabled={isDisabled}
      className={cn(styles, className)}
      {...props}
    />
  );
};
Input.displayName = "Input";

interface IconProps {
  icon: LucideIcon;
  className?: string;
}

/**
 * Icon component that appears at the beginning of an Input.
 * Must be used within an InputContainer.
 *
 * @component
 * @param {LucideIcon} props.icon - The icon component to render
 * @param {string} [props.className] - Additional CSS classes
 *
 * @example
 * <InputContainer>
 *   <InputLeadIcon icon={User} />
 *   <Input placeholder="Username" />
 * </InputContainer>
 */
function InputLeadIcon({ icon, className }: IconProps) {
  const inputContainerContext = useContext(InputContainerContext);

  if (!inputContainerContext) {
    throw new Error("InputLeadIcon must be used within an InputContainer");
  }

  return createElement(icon, {
    className: inputContainerContext.styles.leadIcon({ className }),
  });
}
InputLeadIcon.displayName = "InputLeadIcon";

/**
 * Icon component that appears at the end of an Input.
 * Must be used within an InputContainer.
 *
 * @component
 * @param {LucideIcon} props.icon - The icon component to render
 * @param {string} [props.className] - Additional CSS classes
 *
 * @example
 * <InputContainer>
 *   <Input placeholder="Information" />
 *   <InputTailIcon icon={Info} />
 * </InputContainer>
 */
function InputTailIcon({ icon, className }: IconProps) {
  const inputContainerContext = useContext(InputContainerContext);

  if (!inputContainerContext) {
    throw new Error("InputTailIcon must be used within an InputContainer");
  }

  return createElement(icon, {
    className: inputContainerContext.styles.tailIcon({ className }),
  });
}
InputTailIcon.displayName = "InputTailIcon";

/**
 * Button component that appears at the end of an Input.
 * Must be used within an InputContainer.
 *
 * @component
 * @param {React.ReactNode} props.children - Button content
 * @param {string} [props.className] - Additional CSS classes
 * @param {Function} [props.onClick] - Click handler function
 * @param {string} [props.size='sm'] - Size of the button
 *
 * @example
 * <InputContainer>
 *   <Input placeholder="Search..." />
 *   <InputTailButton onClick={() => console.log("Search clicked")}>
 *     Search
 *   </InputTailButton>
 * </InputContainer>
 *
 * @example
 * // Button with icon
 * <InputContainer>
 *   <Input placeholder="Type a message..." />
 *   <InputTailButton>
 *     <Send className="size-4" />
 *   </InputTailButton>
 * </InputContainer>
 */
function InputTailButton({ className, ...props }: ButtonProps) {
  const inputContainerContext = useContext(InputContainerContext);

  if (!inputContainerContext) {
    throw new Error("InputTailButton must be used within an InputContainer");
  }

  return (
    <Button
      variant="none"
      {...props}
      className={inputContainerContext.styles.tailButton({ className })}
    />
  );
}
InputTailButton.displayName = "InputTailButton";

type ContainerProps = {
  children: React.ReactNode;
} & HTMLAttributes<HTMLDivElement> &
  InputVariantProps;

/**
 * Container component that groups an Input with optional icons and buttons.
 * Provides context for styling and behavior of child components.
 *
 * @component
 * @param {React.ReactNode} props.children - Child components (Input, LeadIcon, TailIcon, TailButton)
 * @param {string} [props.className] - Additional CSS classes
 * @param {'default' | 'soft'} [props.variant='default'] - Visual style variant
 * @param {'sm' | 'lg'} [props.size='lg'] - Size of the input container
 *
 * @example
 * // Input with lead icon and tail button
 * <InputContainer>
 *   <InputLeadIcon icon={Search} />
 *   <Input placeholder="Search..." />
 *   <InputTailButton>Search</InputTailButton>
 * </InputContainer>
 *
 * @example
 * // Soft variant with both icons
 * <InputContainer variant="soft" size="sm">
 *   <InputLeadIcon icon={Mail} />
 *   <Input placeholder="Email address" />
 *   <InputTailIcon icon={Info} />
 * </InputContainer>
 */
function InputContainer({
  className,
  children,
  size,
  variant,
}: ContainerProps) {
  const slots = createSlots(children, {
    input: Input,
    leadIcon: InputLeadIcon,
    tailIcon: InputTailIcon,
    tailButton: InputTailButton,
  });

  const contextValue = useMemo(() => {
    const styles = inputVariants({
      variant,
      className,
      size,
    });
    return { styles, variants: { size, variant } };
  }, [variant, className, size]);

  return (
    <InputContainerContext.Provider value={contextValue}>
      <div
        className={cn(
          contextValue.styles.container(),
          contextValue.styles.wrapper(),
          className,
        )}
      >
        {slots.leadIcon}
        {slots.input}
        {slots.main}
        {slots.tailIcon}
        {slots.tailButton}
      </div>
    </InputContainerContext.Provider>
  );
}
InputContainer.displayName = "InputContainer";

export { Input, InputContainer, InputLeadIcon, InputTailButton, InputTailIcon };
