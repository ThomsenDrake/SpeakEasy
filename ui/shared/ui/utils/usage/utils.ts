export const formatNumberCompact = (num: number) => {
  const absNum = Math.abs(num);

  // Compact notation with maximumFractionDigits: 2 rounds values below 0.01 to "0".
  // Use significant digits instead to preserve meaningful precision on the Y-axis.
  if (absNum > 0 && absNum < 0.01) {
    const formatter = new Intl.NumberFormat(undefined, {
      maximumSignificantDigits: 2,
    });
    return formatter.format(num);
  }

  const formatter = new Intl.NumberFormat(undefined, {
    minimumFractionDigits: 0,
    maximumFractionDigits: 2,
    notation: "compact",
    compactDisplay: "short",
  });

  return formatter.format(num);
};

export const formatChartTooltipValue = (
  value: number | string | Array<number | string>,
  unit: string,
) => {
  const numValue = Number(value);
  if (isNaN(numValue)) return String(value);

  const unitIsCurrency = Intl.supportedValuesOf("currency").includes(unit);

  const absValue = Math.abs(numValue);
  const needsMorePrecision = absValue > 0 && absValue < 0.01;

  const formatter = new Intl.NumberFormat(undefined, {
    ...(needsMorePrecision
      ? { maximumSignificantDigits: 2 }
      : {
          minimumFractionDigits: unitIsCurrency ? 2 : 0,
          maximumFractionDigits: 2,
        }),
    style: unitIsCurrency ? "currency" : undefined,
    currency: unitIsCurrency ? unit : undefined,
  });

  const formattedValue = formatter.format(numValue);

  return unitIsCurrency ? `${formattedValue}` : `${formattedValue} ${unit}`;
};
