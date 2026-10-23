"use client";

import { Label as RadixLabel } from "@radix-ui/react-label";
import { useContext, useId, useMemo, useState } from "react";

import { createSlots } from "#utils/create-slots.tsx";

import { FieldContext } from "../form/form-field-context";
import { TypographySpan } from "../typography";
import { SwitchContainerContext } from "./switch-container-context";
import { type SwitchVariantProps, switchVariants } from "./switch-variants";

export type SwitchProps = Omit<
  React.HTMLAttributes<HTMLInputElement>,
  "onChange"
> &
  Omit<SwitchVariantProps, "position" | "size"> & {
    onCheckedChange?: (isChecked: boolean) => void;
    onChange?: (e: React.ChangeEvent<HTMLInputElement>) => void;
    defaultChecked?: boolean;
    checked?: boolean;
    isDisabled?: boolean;
    children?: undefined;
  };

/**
 * A toggle switch component that allows users to turn options on or off.
 *
 * The Switch component can be used standalone or with labels and descriptions
 * using the compound components pattern (SwitchContainer, SwitchLabel, SwitchDescription).
 *
 * @component
 *
 * @property {boolean} [checked] - Controlled checked state
 * @property {boolean} [defaultChecked] - Initial checked state for uncontrolled component
 * @property {boolean} [isDisabled] - Whether the switch is disabled
 * @property {function} [onChange] - Callback with event when switch state changes (use with `form.register()`)
 * @property {function} [onCheckedChange] - Callback with true/false checked state when switch state changes (use for a controlled Switch)
 * @property {string} [className] - Additional CSS class names for styling the track
 *
 * @example
 * // Basic usage
 * <Switch defaultChecked={true} onChange={(e) => console.log(e.target.checked)} />
 *
 * @example
 * // With container, label and description
 * <SwitchContainer position="left">
 *   <Switch />
 *   <SwitchLabel>Dark mode</SwitchLabel>
 *   <SwitchDescription>Switch between light and dark theme</SwitchDescription>
 * </SwitchContainer>
 */
const Switch = ({
  className,
  isDisabled,
  onChange,
  onCheckedChange,
  defaultChecked,
  ...props
}: SwitchProps) => {
  const implicitId = useId();
  const fieldContext = useContext(FieldContext);
  const switchContainerContext = useContext(SwitchContainerContext);

  const [_isCheckedInternal, setIsCheckedInternal] = useState(
    props.checked ?? defaultChecked ?? false,
  );
  const isChecked = props.checked ?? _isCheckedInternal;

  const styles = switchContainerContext
    ? switchContainerContext.styles
    : switchVariants();

  const id =
    fieldContext?.id ?? switchContainerContext?.id ?? props.id ?? implicitId;

  return (
    <div className={styles.toggleable({ className })}>
      <input
        type="checkbox"
        className={styles.hiddenInput()}
        role="switch"
        data-state={isChecked ? "checked" : "unchecked"}
        aria-checked={isChecked}
        aria-invalid={!!fieldContext?.isInvalid}
        aria-disabled={isDisabled}
        disabled={isDisabled}
        id={id}
        {...props}
        checked={isChecked}
        onChange={(e) => {
          const newInternalChecked = !isChecked;
          setIsCheckedInternal(newInternalChecked);
          onCheckedChange?.(newInternalChecked);
          onChange?.(e);
        }}
      />
      <label htmlFor={id} className={styles.track()}>
        <span className={styles.thumb()} />
      </label>
    </div>
  );
};
Switch.displayName = "Switch";

type ContainerProps = Omit<React.HTMLAttributes<HTMLDivElement>, "id"> &
  SwitchVariantProps;

/**
 * Container component for Switch that provides context and layout for the switch and its label/description.
 *
 * @component
 * @param {React.ReactNode} props.children - Should include Switch and optionally SwitchLabel and/or SwitchDescription
 * @param {string} [props.className] - Additional CSS class names
 * @param {string} [props.position="left"] - Position of the switch relative to the label. Options: "left" | "right"
 * @param {string} [props.id] - ID for the switch element (will be generated if not provided)
 *
 * @example
 * <SwitchContainer position="right">
 *   <Switch />
 *   <SwitchLabel>Enable notifications</SwitchLabel>
 * </SwitchContainer>
 */
function SwitchContainer({
  className,
  children,
  position,
  ...props
}: ContainerProps) {
  const fieldContext = useContext(FieldContext);
  const implicitId = useId();
  const id = fieldContext?.id ?? implicitId;

  const slots = createSlots(children, {
    switch: Switch,
    label: SwitchLabel,
    description: SwitchDescription,
  });

  const contextValue = useMemo(() => {
    const styles = switchVariants();
    return { styles, id };
  }, [id]);

  return (
    <SwitchContainerContext.Provider value={contextValue}>
      <div
        className={contextValue.styles.container({ className, position })}
        {...props}
      >
        {slots.switch}

        <RadixLabel htmlFor={id}>
          {slots.label}
          {slots.description}
        </RadixLabel>
      </div>
    </SwitchContainerContext.Provider>
  );
}
SwitchContainer.displayName = "SwitchContainer";

/**
 * Label component for the Switch. Must be used within a SwitchContainer.
 *
 * @component
 * @param {Object} props - The component props
 * @param {React.ReactNode} props.children - The label text content
 * @param {string} [props.className] - Additional CSS class names
 *
 * @example
 * <SwitchContainer>
 *   <Switch />
 *   <SwitchLabel>Enable dark mode</SwitchLabel>
 * </SwitchContainer>
 */
function SwitchLabel({ className, children }: ContainerProps) {
  const switchContainerContext = useContext(SwitchContainerContext);

  if (!switchContainerContext) {
    throw new Error("SwitchLabel must be used within a SwitchContainer");
  }

  return (
    <TypographySpan
      size="sm"
      weight="medium"
      className={switchContainerContext.styles.label({ className })}
    >
      {children}
    </TypographySpan>
  );
}
SwitchLabel.displayName = "SwitchLabel";

/**
 * Description component for the Switch. Must be used within a SwitchContainer.
 * Provides additional information about the switch's function.
 *
 * @component
 * @param {Object} props - The component props
 * @param {React.ReactNode} props.children - The description text content
 * @param {string} [props.className] - Additional CSS class names
 *
 * @example
 * <SwitchContainer>
 *   <Switch />
 *   <SwitchLabel>Auto-save</SwitchLabel>
 *   <SwitchDescription>Automatically save your changes as you work</SwitchDescription>
 * </SwitchContainer>
 */
function SwitchDescription({ className, children }: ContainerProps) {
  const switchContainerContext = useContext(SwitchContainerContext);

  if (!switchContainerContext) {
    throw new Error("SwitchDescription must be used within a SwitchContainer");
  }

  return (
    <TypographySpan
      size="sm"
      variant="subtle"
      className={switchContainerContext.styles.description({ className })}
    >
      {children}
    </TypographySpan>
  );
}
SwitchDescription.displayName = "SwitchDescription";

export { Switch, SwitchContainer, SwitchDescription, SwitchLabel };
