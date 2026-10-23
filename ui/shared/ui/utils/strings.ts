let cachedContext: CanvasRenderingContext2D | null = null;

function getContext(): CanvasRenderingContext2D | null {
  if (cachedContext) return cachedContext;
  if (typeof window === "undefined") return null;
  const canvas = document.createElement("canvas");
  cachedContext = canvas.getContext("2d");
  return cachedContext;
}

const DEFAULT_FONT_STYLE = {
  fontFamily: "Inter, system-ui, sans-serif",
  fontSize: "12px",
  fontWeight: "normal",
};

export type FontStyle = Partial<typeof DEFAULT_FONT_STYLE>;

/**
 * Return the width of a string in pixels, given a font size and font family.
 */
export function getTextWidth(text: string, stylesProp?: FontStyle): number {
  const context = getContext();
  if (!context) return 0;
  const styles = stylesProp
    ? { ...DEFAULT_FONT_STYLE, ...stylesProp }
    : DEFAULT_FONT_STYLE;
  context.font = `${styles.fontWeight} ${styles.fontSize} ${styles.fontFamily}`;
  const metrics = context.measureText(text);
  return metrics.width;
}

/**
 * Trim a string to a given width in pixels, given a font size and font family.
 * @param text The text to trim
 * @param maxWidth Maximum width in pixels
 * @param stylesProp The style of the rendered text to calculate the width
 * @param preserveWords If true, the last word will be removed to avoid trimming in the middle of a word
 * @returns The trimmed text
 */
export function truncateText({
  maxWidth,
  preserveWords,
  stylesProp,
  text,
}: {
  text: string;
  maxWidth: number;
  stylesProp: FontStyle;
  preserveWords?: boolean;
}): string {
  let left = 0;
  let right = text.length;
  let cutPoint = Math.floor((left + right) / 2);

  while (left < right) {
    const snippet = text.slice(0, cutPoint);
    const width = getTextWidth(snippet, stylesProp);
    if (width <= maxWidth) {
      left = cutPoint + 1;
    } else {
      right = cutPoint;
    }
    cutPoint = Math.floor((left + right) / 2);
  }

  const trimedText = text.slice(0, cutPoint);
  if (!preserveWords) return trimedText;

  return trimedText.split(" ").slice(0, -2).join(" ");
}
