interface HSL {
  h: number;
  s: number;
  l: number;
}

interface Shade {
  name: string | number;
  lightness: number;
}

type BrandPalette = Record<string | number, string>;

const hexToHSL = (hex: string): HSL => {
  const match = /^#?([a-f\d]{2})([a-f\d]{2})([a-f\d]{2})$/i.exec(hex) || [];
  let r = parseInt(match[1] ?? "0", 16);
  let g = parseInt(match[2] ?? "0", 16);
  let b = parseInt(match[3] ?? "0", 16);
  r /= 255;
  g /= 255;
  b /= 255;
  const max = Math.max(r, g, b),
    min = Math.min(r, g, b);
  let h = 0;
  let s;
  const l = (max + min) / 2;
  if (max === min) {
    h = s = 0;
  } else {
    const d = max - min;
    s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
    switch (max) {
      case r:
        h = (g - b) / d + (g < b ? 6 : 0);
        break;
      case g:
        h = (b - r) / d + 2;
        break;
      case b:
        h = (r - g) / d + 4;
        break;
    }
    h /= 6;
  }
  const result: HSL = {
    h: Math.round(h * 360),
    s: Math.round(s * 100),
    l: Math.round(l * 100),
  };
  return result;
};

const hslToHex = ({ h, s, l }: HSL): string => {
  l /= 100;
  const a = (s * Math.min(l, 1 - l)) / 100;
  const f = (n: number) => {
    const k = (n + h / 30) % 12;
    const color = l - a * Math.max(Math.min(k - 3, 9 - k, 1), -1);
    return Math.round(255 * color)
      .toString(16)
      .padStart(2, "0");
  };
  return `#${f(0)}${f(8)}${f(4)}`;
};

const BRAND_SHADES: Shade[] = [
  { name: "50", lightness: 98 },
  { name: "100", lightness: 95 },
  { name: "200", lightness: 90 },
  { name: "300", lightness: 82 },
  { name: "400", lightness: 64 },
  { name: "500", lightness: 46 },
  { name: "600", lightness: 33 },
  { name: "700", lightness: 24 },
  { name: "800", lightness: 14 },
  { name: "900", lightness: 7 },
  { name: "950", lightness: 4 },
];

/**
 * Generates a full 11-shade brand palette (50–950) from a single hex color.
 *
 * The algorithm preserves the input color's hue and saturation, varying only
 * lightness to produce each shade. The original hex is preserved at the shade
 * whose target lightness is closest to its actual lightness.
 */
export function generateBrandPalette(hex: string): BrandPalette {
  const colorHSL = hexToHSL(hex);
  const lightnessDelta: Record<string | number, number> = {};

  const palette: BrandPalette = BRAND_SHADES.reduce(
    (obj: BrandPalette, { name, lightness }) => {
      const { h, s } = colorHSL;
      const hsl: HSL = { h, s, l: lightness };
      obj[name] = hslToHex(hsl);
      lightnessDelta[name] = Math.abs(colorHSL.l - lightness);
      return obj;
    },
    {},
  );

  // Preserve the original hex at the closest shade
  const [closestShade] = Object.keys(lightnessDelta).sort(
    (a, b) => (lightnessDelta[a] ?? 0) - (lightnessDelta[b] ?? 0),
  );
  if (closestShade) {
    palette[closestShade] = hex;
  }

  return palette;
}
