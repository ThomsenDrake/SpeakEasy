import { generateBrandPalette } from "./generate-brand-palette";

export function BrandColorsOverride({
  color,
}: {
  color: string | null | undefined;
}) {
  if (!color) return null;

  const colors = generateBrandPalette(color);

  return (
    <style
      dangerouslySetInnerHTML={{
        __html: `
          :root {
            ${Object.entries(colors)
              .map(([key, value]) => `--brand-${key}: ${value};`)
              .join("\n")}
          }
        `,
      }}
    />
  );
}
