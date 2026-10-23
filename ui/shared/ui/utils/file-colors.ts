/**
 * Shared file color utilities for web and mobile.
 * Provides consistent color mappings for file types and extensions.
 */

import { tv } from "tailwind-variants";

export type FileType =
  | "image"
  | "document"
  | "spreadsheet"
  | "text"
  | "audio"
  | "other";

/**
 * Color scheme names used by Badge component.
 */
export type ColorScheme =
  | "neutral"
  | "red"
  | "orange"
  | "orangebright"
  | "yellow"
  | "lime"
  | "green"
  | "emerald"
  | "teal"
  | "cyan"
  | "sky"
  | "blue"
  | "bluedeep"
  | "purpledeep"
  | "purple"
  | "pinkdeep"
  | "pink"
  | "rose";

/**
 * Tailwind background and text classes for each file type.
 */
export const fileTypeVariants = tv({
  variants: {
    bg: {
      image: "bg-basic-yellow-accent",
      document: "bg-basic-rose-accent",
      spreadsheet: "bg-basic-emerald-accent",
      text: "bg-basic-gray-accent",
      audio: "bg-basic-orangebright-accent",
      other: "bg-basic-gray-accent",
    },
    text: {
      image: "text-white-default",
      document: "text-white-default",
      spreadsheet: "text-white-default",
      text: "text-white-default",
      audio: "text-white-default",
      other: "text-white-default",
    },
  },
});

/**
 * Background classes for each file type.
 * These classes work on both web and mobile (NativeWind).
 */
export const FILE_TYPE_BG_COLORS = fileTypeVariants.variants.bg;

/**
 * Text classes for each file type (for badges/tags).
 */
export const FILE_TYPE_TEXT_COLORS = fileTypeVariants.variants.text;

/**
 * Badge colorScheme values for each file type (for web Badge component).
 */
export const FILE_TYPE_COLOR_SCHEMES: Record<FileType, ColorScheme> = {
  image: "yellow",
  document: "rose",
  spreadsheet: "emerald",
  text: "neutral",
  audio: "orangebright",
  other: "neutral",
};

/**
 * Extension-level color schemes (for granular web Badge colors).
 */
export const EXTENSION_COLOR_SCHEMES: Record<string, ColorScheme> = {
  py: "bluedeep",
  js: "yellow",
  ts: "blue",
  md: "neutral",
  html: "orange",
  css: "blue",
  json: "emerald",
  jsonl: "emerald",
  xml: "green",
  yaml: "yellow",
  toml: "yellow",
  ini: "neutral",
  txt: "neutral",
  docx: "blue",
  pdf: "rose",
  pptx: "orange",
  eml: "neutral",
  msg: "neutral",
  tex: "neutral",
  java: "orangebright",
  kt: "bluedeep",
  scala: "rose",
  groovy: "green",
  c: "neutral",
  cpp: "bluedeep",
  cs: "purple",
  php: "purpledeep",
  rb: "rose",
  swift: "orangebright",
  m: "yellow",
  go: "sky",
  rs: "orange",
  zig: "orange",
  nim: "yellow",
  clj: "green",
  hs: "purpledeep",
  erl: "red",
  ml: "orange",
  fs: "bluedeep",
  elm: "teal",
  lua: "blue",
  jl: "teal",
  pl: "bluedeep",
  r: "blue",
  dart: "sky",
  sh: "green",
  ps1: "bluedeep",
  bat: "neutral",
  coffee: "yellow",
  rkt: "purple",
  sol: "cyan",
  vy: "green",
  cr: "red",
  v: "bluedeep",
  d: "red",
  re: "orangebright",
  purs: "purple",
  idr: "bluedeep",
  agda: "purpledeep",
  coq: "purpledeep",
  lean: "bluedeep",
  red: "red",
  factor: "green",
  forth: "yellow",
  st: "purple",
  apl: "cyan",
  j: "blue",
  k: "cyan",
  q: "blue",
  sql: "cyan",
  glsl: "orange",
  hlsl: "orangebright",
  wasm: "yellow",
  graphql: "pinkdeep",
  ahk: "green",
  au3: "green",
  vbs: "purple",
  logo: "green",
  pde: "blue",
  sb: "orange",
  abap: "bluedeep",
  ada: "neutral",
  apex: "bluedeep",
  asm: "red",
  cob: "neutral",
  ex: "purple",
  f90: "orange",
  lisp: "purpledeep",
  pas: "yellow",
  plsql: "orangebright",
  tsql: "cyan",
  vb: "blue",
  less: "blue",
  scss: "pink",
  sass: "pinkdeep",
  styl: "purple",
  mp3: "purple",
  wav: "purple",
  m4a: "purple",
  ogg: "purple",
  flac: "purple",
} as const;

/**
 * Get file type colors as an object with bg and text classes.
 */
export const getFileTypeColors = (fileType: FileType) => ({
  // safe fallback
  // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
  bg: FILE_TYPE_BG_COLORS[fileType] ?? FILE_TYPE_BG_COLORS.other,
  // safe fallback
  // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
  text: FILE_TYPE_TEXT_COLORS[fileType] ?? FILE_TYPE_TEXT_COLORS.other,
});

/**
 * Get badge colorScheme for an extension, falling back to file type.
 */
export const getExtensionColorScheme = (
  extension: string | undefined,
  fallbackType: FileType = "other",
): ColorScheme => {
  if (extension) {
    const scheme = EXTENSION_COLOR_SCHEMES[extension.toLowerCase()];
    if (scheme) return scheme;
  }
  return FILE_TYPE_COLOR_SCHEMES[fallbackType];
};

/**
 * Map color scheme names to Tailwind bg-basic-*-accent classes.
 * Using tv() for Tailwind class linting support.
 */
const colorSchemeBgVariants = tv({
  variants: {
    colorScheme: {
      neutral: "bg-basic-gray-accent",
      red: "bg-basic-red-accent",
      orange: "bg-basic-orange-accent",
      orangebright: "bg-basic-orangebright-accent",
      yellow: "bg-basic-yellow-accent",
      lime: "bg-basic-lime-accent",
      green: "bg-basic-green-accent",
      emerald: "bg-basic-emerald-accent",
      teal: "bg-basic-teal-accent",
      cyan: "bg-basic-cyan-accent",
      sky: "bg-basic-sky-accent",
      blue: "bg-basic-blue-accent",
      bluedeep: "bg-basic-bluedeep-accent",
      purpledeep: "bg-basic-purpledeep-accent",
      purple: "bg-basic-purple-accent",
      pinkdeep: "bg-basic-pinkdeep-accent",
      pink: "bg-basic-pink-accent",
      rose: "bg-basic-rose-accent",
    },
  },
});

const COLOR_SCHEME_TO_BG_CLASS = colorSchemeBgVariants.variants.colorScheme;

/**
 * Get Tailwind bg class for an extension.
 * Returns a bg-basic-*-accent class for use in custom components.
 */
export const getExtensionBgClass = (
  extension: string | undefined,
  fallbackType: FileType = "other",
): string => {
  const colorScheme = getExtensionColorScheme(extension, fallbackType);
  return COLOR_SCHEME_TO_BG_CLASS[colorScheme];
};
