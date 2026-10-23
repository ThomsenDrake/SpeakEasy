"use client";

import { Label as RadixLabel } from "@radix-ui/react-label";
import type { ChangeEventHandler } from "react";
import { useContext, useId, useMemo, useState } from "react";

import { createSlots } from "#utils/create-slots.tsx";

import { FieldContext } from "../form/form-field-context";
import { TypographySpan } from "../typography/typography";
import { RadioContainerContext } from "./radio-container-context";
import { RadioGroupContext } from "./radio-group-context";
import { type RadioVariantProps, radioVariants } from "./radio-variants";

export type RadioProps = Omit<
  React.HTMLAttributes<HTMLInputElement>,
  "onChange"
> &
  Omit<RadioVariantProps, "position"> & {
    onCheckedChange?: (isChecked: boolean) => void;
    onChange?: ChangeEventHandler<HTMLInputElement>;
    defaultChecked?: boolean;
    checked?: boolean;
    isDisabled?: boolean;
    children?: undefined;
    value: string;
  };

/**
 * A customizable radio component with support for labels, descriptions, and different styling variants.
 *
 * @component
 * @param {string} [className] - Additional CSS class names
 * @param {boolean} [isDisabled] - Whether the radio is disabled
 * @param {boolean} [defaultChecked] - Initial checked state (uncontrolled)
 * @param {boolean} [checked] - Controlled checked state
 * @param {Function} [onChange] - Handler called when the radio's value changes
 * @param {Function} [onCheckedChange] - Alternative handler that receives only the boolean checked state
 * @param {'default' | 'with-shadow'} [variant='default'] - Visual style variant
 *
 * @example
 * // Basic usage
 * <Radio defaultChecked={true} />
 *
 * @example
 * // Controlled component with shadow variant
 * const [isChecked, setIsChecked] = useState(false);
 *
 * <Radio
 *   checked={isChecked}
 *   onChange={(e) => setIsChecked(e.target.checked)}
 *   variant="with-shadow"
 * />
 */
const Radio = ({
  className,
  isDisabled: _isDisabled,
  onChange,
  onCheckedChange,
  variant,
  value,
  ...props
}: RadioProps) => {
  const implicitId = useId();
  const fieldContext = useContext(FieldContext);
  const radioContainerContext = useContext(RadioContainerContext);
  const radioGroupContext = useContext(RadioGroupContext);
  const isDisabled = radioGroupContext?.isDisabled || _isDisabled;

  const [_isCheckedInternal, setIsCheckedInternal] = useState(
    radioGroupContext
      ? radioGroupContext.value === value
      : Boolean(props.checked ?? props.defaultChecked ?? false),
  );

  const isChecked = radioGroupContext
    ? radioGroupContext.value === value
    : (props.checked ?? _isCheckedInternal);

  const styles = radioContainerContext
    ? radioContainerContext.styles
    : radioGroupContext
      ? radioGroupContext.styles
      : radioVariants({ variant });

  const id = radioContainerContext?.id ?? props.id ?? implicitId;

  return (
    <div className={styles.toggleable({ className })}>
      <input
        type="radio"
        className={styles.hiddenInput()}
        role="radio"
        checked={isChecked}
        data-state={isChecked ? "checked" : "unchecked"}
        aria-checked={isChecked}
        aria-invalid={!!fieldContext?.isInvalid}
        aria-disabled={isDisabled}
        disabled={isDisabled}
        id={id}
        {...props}
        // onClick instead of onChange because we want a radio button to be deselectable
        onClick={() => {
          const newChecked = !isChecked;
          setIsCheckedInternal(newChecked);
          onCheckedChange?.(newChecked);
          radioGroupContext?.onValueChange(value);
        }}
        onChange={(e) => {
          radioGroupContext?.onChange?.(e);
          onChange?.(e);
        }}
      />
      <label htmlFor={id} className={styles.radio()}>
        <span className={styles.innerCircle()}></span>
        <span className={styles.phantomCircle()}></span>
      </label>
    </div>
  );
};
Radio.displayName = "Radio";

type GroupProps = Omit<
  React.HTMLAttributes<HTMLDivElement>,
  "id" | "onChange"
> &
  RadioVariantProps & {
    isDisabled?: boolean;
    defaultValue?: string;
    value: string;
    onValueChange: (value: string) => void;
    onChange?: ChangeEventHandler<HTMLInputElement>;
    ref?: React.Ref<HTMLDivElement>;
  };

/**
 * Group component for Radio that provides layout and context for radio options.
 *
 * @component
 * @param {string} [className] - Additional CSS class names
 * @param {React.ReactNode} props.children - Child components (Radio)
 * @param {'default' | 'with-shadow'} [variant='default'] - Visual style variant
 * @param {boolean} [isDisabled] - Whether the group is disabled
 * @param {string} [defaultValue] - Initial value of the selected radio option
 * @param {string} [value] - Controlled value of the selected radio option
 * @param {(value: string) => void} [onValueChange] - Handler called when the selected radio option changes
 *
 * @warning only supports controlled mode with `value` and `onValueChange` for now.
 *
 * @example
 * <RadioGroup>
 *   <RadioContainer>
 *     <Radio value="option1" />
 *     <RadioLabel>Option 1</RadioLabel>
 *     <RadioDescription>First option description</RadioDescription>
 *   </RadioContainer>
 *   <RadioContainer>
 *     <Radio value="option2" />
 *     <RadioLabel>Option 2</RadioLabel>
 *     <RadioDescription>Second option description</RadioDescription>
 *   </RadioContainer>
 * </RadioGroup>
 */
function RadioGroup({
  children,
  className,
  variant,
  value,
  onValueChange,
  onChange,
  isDisabled,
  // we don't spread ref on the div because it breaks react-hook-form (we don't have a ref on an input here)
  ref: _ref,
  ...props
}: GroupProps) {
  const contextValue = useMemo(() => {
    const styles = radioVariants({ variant });
    return { styles, value, onValueChange, onChange, isDisabled };
  }, [variant, value, onValueChange, onChange, isDisabled]);

  return (
    <RadioGroupContext.Provider value={contextValue}>
      <div
        role="radiogroup"
        className={contextValue.styles.group({ className })}
        {...props}
      >
        {children}
      </div>
    </RadioGroupContext.Provider>
  );
}

type ContainerProps = Omit<React.HTMLAttributes<HTMLDivElement>, "id"> &
  RadioVariantProps;

/**
 * Container component for Radio that provides layout and context for label and description.
 *
 * @component
 * @param {string} [className] - Additional CSS class names
 * @param {React.ReactNode} props.children - Child components (Radio, Label, Description)
 * @param {'left' | 'right'} [position='left'] - Position of the Radio relative to label/description
 * @param {'default' | 'with-shadow'} [variant='default'] - Visual style variant
 *
 * @example
 * // Basic usage with label and description
 * <RadioContainer>
 *   <Radio />
 *   <RadioLabel>Enable notifications</RadioLabel>
 *   <RadioDescription>
 *     Receive notifications for new messages and updates
 *   </RadioDescription>
 * </RadioContainer>
 *
 * @example
 * // With right positioning (radio on the right side)
 * <RadioContainer position="right">
 *   <Radio defaultChecked />
 *   <RadioLabel>Auto-save</RadioLabel>
 *   <RadioDescription>
 *     Automatically save your changes as you work
 *   </RadioDescription>
 * </RadioContainer>
 */
function RadioContainer({
  className,
  children,
  position,
  ...props
}: ContainerProps) {
  const radioGroupContext = useContext(RadioGroupContext);
  const implicitId = useId();
  const id = implicitId;

  const slots = createSlots(children, {
    radio: Radio,
    label: RadioLabel,
    description: RadioDescription,
  });

  const contextValue = useMemo(() => {
    const styles = radioGroupContext?.styles ?? radioVariants();
    return { styles, id };
  }, [radioGroupContext?.styles, id]);

  return (
    <RadioContainerContext.Provider value={contextValue}>
      <div
        className={contextValue.styles.container({ className, position })}
        {...props}
      >
        {slots.radio}

        <RadixLabel htmlFor={id}>
          {slots.label}
          {slots.description}
        </RadixLabel>
      </div>
    </RadioContainerContext.Provider>
  );
}
RadioContainer.displayName = "RadioContainer";

/**
 * Label component for the Radio. Must be used within a RadioContainer.
 *
 * @component
 * @param {string} [className] - Additional CSS class names
 * @param {React.ReactNode} props.children - Label content
 *
 * @example
 * <RadioContainer>
 *   <Radio />
 *   <RadioLabel>Dark mode</RadioLabel>
 * </RadioContainer>
 */
function RadioLabel({ className, children }: ContainerProps) {
  const radioContainerContext = useContext(RadioContainerContext);

  if (!radioContainerContext) {
    throw new Error("RadioLabel must be used within a RadioContainer");
  }

  return (
    <TypographySpan
      size="sm"
      weight="medium"
      className={radioContainerContext.styles.label({ className })}
    >
      {children}
    </TypographySpan>
  );
}
RadioLabel.displayName = "RadioLabel";

/**
 * Description component for the Radio. Must be used within a RadioContainer.
 * Provides additional information about the radio option.
 *
 * @component
 * @param {string} [className] - Additional CSS class names
 * @param {React.ReactNode} props.children - Description content
 *
 * @example
 * <RadioContainer>
 *   <Radio />
 *   <RadioLabel>Data collection</RadioLabel>
 *   <RadioDescription>
 *     Allow anonymous usage data to be collected to improve our services
 *   </RadioDescription>
 * </RadioContainer>
 */
function RadioDescription({ className, children }: ContainerProps) {
  const radioContainerContext = useContext(RadioContainerContext);

  if (!radioContainerContext) {
    throw new Error("RadioDescription must be used within a RadioContainer");
  }

  return (
    <TypographySpan
      size="sm"
      variant="subtle"
      className={radioContainerContext.styles.description({ className })}
    >
      {children}
    </TypographySpan>
  );
}
RadioDescription.displayName = "RadioDescription";

export { Radio, RadioContainer, RadioDescription, RadioGroup, RadioLabel };
