"use client";

/* eslint-disable @typescript-eslint/no-explicit-any */
import { AnimatePresence, motion } from "framer-motion";
import {
  type ComponentProps,
  type HTMLAttributes,
  type ReactNode,
  useContext,
  useId,
  useMemo,
} from "react";
import * as React from "react";
import type {
  ControllerProps,
  FieldPath,
  FieldValues,
  UseFormReturn,
} from "react-hook-form";
import { Controller, FormProvider, get } from "react-hook-form";

import { createSlots } from "#utils/create-slots.tsx";

import { Label as BaseLabel } from "../label/label";
import { Typography } from "../typography/typography";
import { FieldContext } from "./form-field-context";
import { formFieldVariants } from "./form-field-variants";

type HtmlFormProps = React.DetailedHTMLProps<
  React.FormHTMLAttributes<HTMLFormElement>,
  HTMLFormElement
>;

interface FormProps<
  TFieldValues extends FieldValues,
  TContext = unknown,
  TTransformedValues extends FieldValues | undefined = undefined,
> extends HtmlFormProps {
  onSubmit: Exclude<HtmlFormProps["onSubmit"], undefined>;
  form: UseFormReturn<TFieldValues, TContext, TTransformedValues>;
}

/**
 * Form component that provides context and handles form submission.
 * Must be used with a `react-hook-form` form.
 *
 * @component
 * @param {Function} props.onSubmit - Form submission handler
 * @param {UseFormReturn<TFieldValues, TContext, TTransformedValues>} props.form - form prop coming from `useForm()`
 * @param {React.ReactNode} props.children - Form children
 * @param {string} [props.className] - Additional CSS classes
 *
 * @example
 * // Example usage of Form
 * const form = useForm()
 * ...
 * <Form onSubmit={onSubmit} form={form} className="space-y-4">
 *   <FormField name="email" form={form}>
 *     <FormLabel>Email</FormLabel>
 *       <Input
 *         placeholder="you@example.com"
 *         {...form.register("email")}
 *       />
 *     <FormHint>We'll send a confirmation to this address</FormHint>
 *   </FormField>
 * </Form>
 */
const Form = <
  TFieldValues extends FieldValues,
  TContext = unknown,
  TTransformedValues extends FieldValues | undefined = undefined,
>({
  form,
  children,
  ...props
}: FormProps<TFieldValues, TContext, TTransformedValues>) => (
  <FormProvider {...form}>
    <form {...props}>
      {children}
      {/* To allow Enter keypress to submit if there's no submit button */}
      <button type="submit" className="hidden" />
    </form>
  </FormProvider>
);

type FieldProps<
  TForm extends FieldValues,
  TName extends FieldPath<TForm>,
> = Omit<HTMLAttributes<HTMLDivElement>, "children"> & {
  form: UseFormReturn<TForm, any, any>;
  name: TName;
  hideError?: boolean;
};

const FieldInternal = <
  TForm extends FieldValues,
  TName extends FieldPath<TForm>,
>({
  className,
  form,
  name,
  children,
  id: explicitId,
  hideError,
  ...props
}: FieldProps<TForm, TName> & {
  children: React.ReactNode;
}) => {
  const implicitId = useId();
  const id = explicitId ?? implicitId;

  const fieldErrors = get(form.formState.errors, name);
  const error = hideError
    ? undefined
    : fieldErrors?.message || fieldErrors?.root?.message;

  const slots = createSlots(children, {
    label: FormLabel,
    hint: FormHint,
  });

  const contextValue = useMemo(() => {
    const styles = formFieldVariants();
    return { id, name, isInvalid: !!error, styles };
  }, [id, name, error]);

  return (
    <FieldContext.Provider value={contextValue}>
      <div className={contextValue.styles.container({ className })} {...props}>
        {slots.label}

        {slots.main}

        <AnimatePresence initial={false}>
          {(error || slots.hint) && (
            <motion.div
              key="message-container"
              initial={{ height: 0 }}
              animate={{ height: "auto" }}
              exit={{ height: 0 }}
            >
              {error ? (
                <motion.div
                  key="error"
                  initial={{ opacity: 0, y: 3 }}
                  animate={{ opacity: 1, y: 0 }}
                  exit={{ opacity: 0, y: 3 }}
                >
                  <Typography size="xs" variant="destructive" className="pt-1">
                    {error}
                  </Typography>
                </motion.div>
              ) : (
                slots.hint && (
                  <motion.div
                    key="hint"
                    initial={{ opacity: 0, y: 3 }}
                    animate={{ opacity: 1, y: 0 }}
                    exit={{ opacity: 0, y: 3 }}
                  >
                    {slots.hint}
                  </motion.div>
                )
              )}
            </motion.div>
          )}
        </AnimatePresence>
      </div>
    </FieldContext.Provider>
  );
};

/**
 * Field component that provides context and handles field rendering.
 *
 * @component
 * @template TForm - The type of form values
 * @template TName - The type of field name
 * @param {UseFormReturn<TForm, any, any>} props.form - Form methods and state
 * @param {TName} props.name - Field name
 * @param {React.ReactNode | (() => React.ReactNode)} props.children - Field children or render function that receives controller props (for controlled components)
 *
 * @example
 * // Example usage of FormField
 * <FormField name="company" form={form}>
 *   <FormLabel>Company</FormLabel>
 *   <Input
 *     className="w-full"
 *     placeholder="Acme Inc."
 *     {...form.register("company")}
 *   />
 *   <FormHint>Optional</FormHint>
 * </FormField>
 */
const FormField = <TForm extends FieldValues, TName extends FieldPath<TForm>>({
  form,
  name,
  children,
  ...props
}: FieldProps<TForm, TName> & {
  children:
    | React.ReactNode
    | ((
        props: Parameters<ControllerProps<TForm, TName>["render"]>[0],
      ) => React.ReactElement);
}) => {
  return typeof children === "function" ? (
    <Controller
      control={form.control}
      name={name}
      render={(controllerProps) => (
        <FieldInternal form={form} name={name} {...props}>
          {children(controllerProps)}
        </FieldInternal>
      )}
    />
  ) : (
    <FieldInternal form={form} name={name} {...props}>
      {children}
    </FieldInternal>
  );
};
FormField.displayName = "FormField";

/**
 * Label component that provides context and handles label rendering.
 * Must be used within a FormField.
 *
 * @component
 * @param {string} [props.className] - Additional CSS classes
 * @param {React.ReactNode} props.children - Text for the label
 */
function FormLabel({
  className,
  children,
  htmlFor,
}: ComponentProps<typeof BaseLabel>) {
  const fieldContext = useContext(FieldContext);

  if (!fieldContext) {
    throw new Error("FormLabel must be used within a FormField");
  }

  return (
    <BaseLabel
      className={fieldContext.styles.label({ className })}
      htmlFor={htmlFor ?? fieldContext.id}
    >
      {children}
    </BaseLabel>
  );
}
FormLabel.displayName = "FormLabel";

interface HintProps {
  className?: string;
  children: ReactNode;
}

/**
 * Hint component that provides context and handles hint rendering.
 * Replaced by an error message if the field is invalid.
 * Must be used within a FormField.
 *
 * @component
 * @param {string} [props.className] - Additional CSS classes
 * @param {React.ReactNode} props.children - Text for the hint
 */
function FormHint({ className, children }: HintProps) {
  const fieldContext = useContext(FieldContext);

  if (!fieldContext) {
    throw new Error("FormHint must be used within a FormField");
  }

  return (
    <Typography className={fieldContext.styles.hint({ className })}>
      {children}
    </Typography>
  );
}
FormHint.displayName = "FormHint";

export { Form, FormField, FormHint, FormLabel };
