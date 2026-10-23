"use client";

import { Label as RadixLabel } from "@radix-ui/react-label";
import { motion } from "framer-motion";
import { useContext, useId, useMemo, useState } from "react";

import { createSlots } from "#utils/create-slots.tsx";

import { FieldContext } from "../form/form-field-context";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "../tooltip/tooltip";
import { TypographySpan } from "../typography/typography";
import { CheckboxContainerContext } from "./checkbox-container-context";
import {
  checkboxMotion,
  type CheckboxVariantProps,
  checkboxVariants,
} from "./checkbox-variants";

export type CheckboxProps = Omit<
  React.HTMLAttributes<HTMLInputElement>,
  "onChange"
> &
  Omit<CheckboxVariantProps, "position"> & {
    onCheckedChange?: (isChecked: boolean) => void;
    onChange?: (e: React.ChangeEvent<HTMLInputElement>) => void;
    defaultChecked?: boolean;
    checked?: boolean;
    isDisabled?: boolean;
    allSelected?: boolean;
    disabledTooltip?: string;
    children?: undefined;
  };

/**
 * A customizable checkbox component with support for labels, descriptions, and different styling variants.
 *
 * @component
 * @param {string} [props.className] - Additional CSS class names
 * @param {boolean} [props.isDisabled] - Whether the checkbox is disabled
 * @param {boolean} [props.defaultChecked] - Initial checked state (uncontrolled)
 * @param {boolean} [props.checked] - Controlled checked state
 * @param {Function} [props.onChange] - Handler called when the checkbox's value changes
 * @param {Function} [props.onCheckedChange] - Alternative handler that receives only the boolean checked state
 * @param {'default' | 'with-shadow'} [props.variant='default'] - Visual style variant
 * @param {boolean} [props.allSelected=true] - When true, shows a checkmark instead of a dash when checked
 * @param {string} [props.disabledTooltip] - Tooltip text to show when checkbox is disabled
 *
 * @example
 * // Basic usage
 * <Checkbox defaultChecked={true} />
 *
 * @example
 * // Controlled component with shadow variant
 * const [isChecked, setIsChecked] = useState(false);
 *
 * <Checkbox
 *   checked={isChecked}
 *   onChange={(e) => setIsChecked(e.target.checked)}
 *   variant="with-shadow"
 * />
 */
const Checkbox = ({
  className,
  isDisabled,
  onChange,
  onCheckedChange,
  variant,
  allSelected = true,
  disabledTooltip,
  ...props
}: CheckboxProps) => {
  const implicitId = useId();
  const fieldContext = useContext(FieldContext);
  const checkboxContainerContext = useContext(CheckboxContainerContext);

  const [_isCheckedInternal, setIsCheckedInternal] = useState(
    props.checked ?? props.defaultChecked ?? false,
  );
  const isChecked = props.checked ?? _isCheckedInternal;

  const styles = checkboxContainerContext
    ? checkboxContainerContext.styles
    : checkboxVariants({ variant });

  const id =
    fieldContext?.id ?? checkboxContainerContext?.id ?? props.id ?? implicitId;

  const checkboxElement = (
    <div className={styles.toggleable({ className })}>
      <input
        type="checkbox"
        className={styles.hiddenInput()}
        role="checkbox"
        checked={isChecked}
        data-state={isChecked ? "checked" : "unchecked"}
        aria-checked={isChecked}
        aria-invalid={!!fieldContext?.isInvalid}
        aria-disabled={isDisabled}
        disabled={isDisabled}
        id={id}
        {...props}
        onChange={(e) => {
          const newInternalChecked = !isChecked;
          setIsCheckedInternal(newInternalChecked);
          onCheckedChange?.(newInternalChecked);
          onChange?.(e);
        }}
      />
      <motion.label
        variants={checkboxMotion.box}
        htmlFor={id}
        className={styles.checkbox()}
        animate={isChecked ? "checked" : "unchecked"}
        initial={false}
      >
        <svg
          xmlns="http://www.w3.org/2000/svg"
          fill="none"
          viewBox="0 0 24 24"
          className={styles.ticks()}
        >
          <motion.path
            strokeLinecap="round"
            strokeLinejoin="round"
            d={allSelected ? "M4.5 12.75l6 6 9-13.5" : "M5 12h14"}
            variants={checkboxMotion.tick}
          />
          <motion.path
            strokeLinecap="round"
            strokeLinejoin="round"
            d={allSelected ? "M4.5 12.75l6 6 9-13.5" : "M5 12h14"}
            strokeWidth={3}
            className={styles.phantomTick()}
            variants={checkboxMotion.phantomTick}
          />
        </svg>
      </motion.label>
    </div>
  );

  if (isDisabled && disabledTooltip) {
    return (
      <TooltipProvider>
        <Tooltip>
          <TooltipTrigger asChild>{checkboxElement}</TooltipTrigger>
          <TooltipContent>{disabledTooltip}</TooltipContent>
        </Tooltip>
      </TooltipProvider>
    );
  }

  return checkboxElement;
};
Checkbox.displayName = "Checkbox";

type ContainerProps = Omit<React.HTMLAttributes<HTMLDivElement>, "id"> &
  CheckboxVariantProps;

/**
 * Container component for Checkbox that provides layout and context for label and description.
 *
 * @component
 * @param {string} [props.className] - Additional CSS class names
 * @param {React.ReactNode} props.children - Child components (Checkbox, Label, Description)
 * @param {'left' | 'right'} [props.position='left'] - Position of the checkbox relative to label/description
 * @param {'default' | 'with-shadow'} [props.variant='default'] - Visual style variant
 *
 * @example
 * // Basic usage with label and description
 * <CheckboxContainer>
 *   <Checkbox />
 *   <CheckboxLabel>Enable notifications</CheckboxLabel>
 *   <CheckboxDescription>
 *     Receive notifications for new messages and updates
 *   </CheckboxDescription>
 * </CheckboxContainer>
 *
 * @example
 * // With right positioning (checkbox on the right side)
 * <CheckboxContainer position="right">
 *   <Checkbox defaultChecked />
 *   <CheckboxLabel>Auto-save</CheckboxLabel>
 *   <CheckboxDescription>
 *     Automatically save your changes as you work
 *   </CheckboxDescription>
 * </CheckboxContainer>
 */
function CheckboxContainer({
  className,
  children,
  position,
  ...props
}: ContainerProps) {
  const fieldContext = useContext(FieldContext);
  const implicitId = useId();
  const id = fieldContext?.id ?? implicitId;

  const slots = createSlots(children, {
    checkbox: Checkbox,
    icon: CheckboxIcon,
    label: CheckboxLabel,
    description: CheckboxDescription,
  });

  const contextValue = useMemo(() => {
    const styles = checkboxVariants();
    return { styles, id };
  }, [id]);

  return (
    <CheckboxContainerContext.Provider value={contextValue}>
      <div
        className={contextValue.styles.container({ className, position })}
        {...props}
      >
        {slots.checkbox}

        <RadixLabel htmlFor={id}>
          {slots.label}
          {slots.description}
        </RadixLabel>

        {slots.icon}
      </div>
    </CheckboxContainerContext.Provider>
  );
}
CheckboxContainer.displayName = "CheckboxContainer";

/**
 * Icon component for the Checkbox. Must be used within a CheckboxContainer.
 *
 * @component
 * @param {string} [props.className] - Additional CSS class names
 * @param {React.ReactNode} props.children - Label content
 *
 * @example
 * <CheckboxContainer>
 *   <CheckboxIcon icon={SunIcon} />
 *   <CheckboxLabel>Dark mode</CheckboxLabel>
 * </CheckboxContainer>
 */
function CheckboxIcon({ className, children }: ContainerProps) {
  const checkboxContainerContext = useContext(CheckboxContainerContext);

  if (!checkboxContainerContext) {
    throw new Error("CheckboxIcon must be used within a CheckboxContainer");
  }

  return (
    <div className={checkboxContainerContext.styles.icon({ className })}>
      {children}
    </div>
  );
}
CheckboxIcon.displayName = "CheckboxIcon";

/**
 * Label component for the Checkbox. Must be used within a CheckboxContainer.
 *
 * @component
 * @param {string} [props.className] - Additional CSS class names
 * @param {React.ReactNode} props.children - Label content
 *
 * @example
 * <CheckboxContainer>
 *   <Checkbox />
 *   <CheckboxLabel>Dark mode</CheckboxLabel>
 * </CheckboxContainer>
 */
function CheckboxLabel({ className, children }: ContainerProps) {
  const checkboxContainerContext = useContext(CheckboxContainerContext);

  if (!checkboxContainerContext) {
    throw new Error("CheckboxLabel must be used within a CheckboxContainer");
  }

  return (
    <TypographySpan
      size="sm"
      weight="medium"
      className={checkboxContainerContext.styles.label({ className })}
    >
      {children}
    </TypographySpan>
  );
}
CheckboxLabel.displayName = "CheckboxLabel";

/**
 * Description component for the Checkbox. Must be used within a CheckboxContainer.
 * Provides additional information about the checkbox option.
 *
 * @component
 * @param {string} [props.className] - Additional CSS class names
 * @param {React.ReactNode} props.children - Description content
 *
 * @example
 * <CheckboxContainer>
 *   <Checkbox />
 *   <CheckboxLabel>Data collection</CheckboxLabel>
 *   <CheckboxDescription>
 *     Allow anonymous usage data to be collected to improve our services
 *   </CheckboxDescription>
 * </CheckboxContainer>
 */
function CheckboxDescription({ className, children }: ContainerProps) {
  const checkboxContainerContext = useContext(CheckboxContainerContext);

  if (!checkboxContainerContext) {
    throw new Error(
      "CheckboxDescription must be used within a CheckboxContainer",
    );
  }

  return (
    <TypographySpan
      size="sm"
      variant="subtle"
      className={checkboxContainerContext.styles.description({ className })}
    >
      {children}
    </TypographySpan>
  );
}
CheckboxDescription.displayName = "CheckboxDescription";

export {
  Checkbox,
  CheckboxContainer,
  CheckboxDescription,
  CheckboxIcon,
  CheckboxLabel,
};
