import type { VariantProps } from "class-variance-authority";
import { tv } from "tailwind-variants";

export const chipVariants = tv({
  slots: {
    base: [
      "flex items-center font-medium transition-all focus-visible:outline-3 outline-default outline-offset-1 justify-center whitespace-nowrap",
    ],
    icon: "",
    group: ["flex"],
  },
  variants: {
    variant: {
      default: {
        base: [
          "bg-state-secondary text-muted active:bg-state-secondary-press border border-darker",
          "hover:bg-state-secondary-hover hover:text-subtle",
          "data-[state=on]:bg-badge-sky data-[state=on]:hover:bg-badge-sky/80 data-[state=on]:active:bg-badge-sky data-[state=on]:border-blue data-[state=on]:text-basic-blue-strong",
        ],
        icon: "in-data-[state=on]:text-basic-blue-strong",
      },
      soft: {
        base: [
          "bg-state-soft text-muted active:bg-state-soft-press",
          "hover:bg-state-soft-hover hover:text-subtle",
          "data-[state=on]:bg-badge-sky data-[state=on]:hover:bg-badge-sky/80 data-[state=on]:active:bg-badge-sky data-[state=on]:text-basic-blue-strong",
        ],
        icon: "in-data-[state=on]:text-basic-blue-strong",
        group: "gap-0.5",
      },
      ghost: {
        base: [
          "bg-transparent text-subtle active:bg-state-ghost-press",
          "hover:bg-state-ghost-hover hover:text-default",
          "data-[state=on]:bg-badge-sky data-[state=on]:hover:bg-badge-sky/80 data-[state=on]:active:bg-badge-sky data-[state=on]:text-basic-blue-strong",
        ],
        icon: "in-data-[state=on]:text-basic-blue-strong",
        group: "gap-0.5",
      },
      brand: {
        base: [
          "bg-transparent text-subtle active:bg-state-ghost-press",
          "hover:bg-state-ghost-hover hover:text-default",
          "data-[state=on]:bg-state-brand/5 data-[state=on]:hover:bg-state-brand-hover/5 data-[state=on]:active:bg-state-brand-press/10 data-[state=on]:text-brand-500 data-[state=on]:border-brand-400/10",
        ],
        icon: "in-data-[state=on]:text-brand-500",
        group: "gap-0.5",
      },
      grey: {
        base: [
          "bg-state-secondary text-muted active:bg-state-secondary-press border border-darker",
          "hover:bg-state-secondary-hover hover:text-subtle",
          "data-[state=on]:bg-state-secondary data-[state=on]:hover:bg-state-secondary-hover data-[state=on]:active:bg-state-secondary-press data-[state=on]:border-default data-[state=on]:text-default",
        ],
        icon: "in-data-[state=on]:text-default",
      },
      // Color variants (default style with border)
      red: {
        base: [
          "bg-state-secondary text-muted active:bg-state-secondary-press border border-darker",
          "hover:bg-state-secondary-hover hover:text-subtle",
          "data-[state=on]:bg-badge-red data-[state=on]:hover:bg-badge-red/80 data-[state=on]:active:bg-badge-red data-[state=on]:border-red data-[state=on]:text-basic-red-strong",
        ],
        icon: "in-data-[state=on]:text-basic-red-strong",
      },
      orange: {
        base: [
          "bg-state-secondary text-muted active:bg-state-secondary-press border border-darker",
          "hover:bg-state-secondary-hover hover:text-subtle",
          "data-[state=on]:bg-badge-orange data-[state=on]:hover:bg-badge-orange/80 data-[state=on]:active:bg-badge-orange data-[state=on]:border-orange data-[state=on]:text-basic-orange-strong",
        ],
        icon: "in-data-[state=on]:text-basic-orange-strong",
      },
      orangebright: {
        base: [
          "bg-state-secondary text-muted active:bg-state-secondary-press border border-darker",
          "hover:bg-state-secondary-hover hover:text-subtle",
          "data-[state=on]:bg-badge-orangebright data-[state=on]:hover:bg-badge-orangebright/80 data-[state=on]:active:bg-badge-orangebright data-[state=on]:border-orangebright data-[state=on]:text-basic-orangebright-strong",
        ],
        icon: "in-data-[state=on]:text-basic-orangebright-strong",
      },
      yellow: {
        base: [
          "bg-state-secondary text-muted active:bg-state-secondary-press border border-darker",
          "hover:bg-state-secondary-hover hover:text-subtle",
          "data-[state=on]:bg-badge-yellow data-[state=on]:hover:bg-badge-yellow/80 data-[state=on]:active:bg-badge-yellow data-[state=on]:border-yellow data-[state=on]:text-basic-yellow-strong",
        ],
        icon: "in-data-[state=on]:text-basic-yellow-strong",
      },
      lime: {
        base: [
          "bg-state-secondary text-muted active:bg-state-secondary-press border border-darker",
          "hover:bg-state-secondary-hover hover:text-subtle",
          "data-[state=on]:bg-badge-lime data-[state=on]:hover:bg-badge-lime/80 data-[state=on]:active:bg-badge-lime data-[state=on]:border-lime data-[state=on]:text-basic-lime-strong",
        ],
        icon: "in-data-[state=on]:text-basic-lime-strong",
      },
      green: {
        base: [
          "bg-state-secondary text-muted active:bg-state-secondary-press border border-darker",
          "hover:bg-state-secondary-hover hover:text-subtle",
          "data-[state=on]:bg-badge-green data-[state=on]:hover:bg-badge-green/80 data-[state=on]:active:bg-badge-green data-[state=on]:border-green data-[state=on]:text-basic-green-strong",
        ],
        icon: "in-data-[state=on]:text-basic-green-strong",
      },
      emerald: {
        base: [
          "bg-state-secondary text-muted active:bg-state-secondary-press border border-darker",
          "hover:bg-state-secondary-hover hover:text-subtle",
          "data-[state=on]:bg-badge-emerald data-[state=on]:hover:bg-badge-emerald/80 data-[state=on]:active:bg-badge-emerald data-[state=on]:border-emerald data-[state=on]:text-basic-emerald-strong",
        ],
        icon: "in-data-[state=on]:text-basic-emerald-strong",
      },
      teal: {
        base: [
          "bg-state-secondary text-muted active:bg-state-secondary-press border border-darker",
          "hover:bg-state-secondary-hover hover:text-subtle",
          "data-[state=on]:bg-badge-teal data-[state=on]:hover:bg-badge-teal/80 data-[state=on]:active:bg-badge-teal data-[state=on]:border-teal data-[state=on]:text-basic-teal-strong",
        ],
        icon: "in-data-[state=on]:text-basic-teal-strong",
      },
      cyan: {
        base: [
          "bg-state-secondary text-muted active:bg-state-secondary-press border border-darker",
          "hover:bg-state-secondary-hover hover:text-subtle",
          "data-[state=on]:bg-badge-cyan data-[state=on]:hover:bg-badge-cyan/80 data-[state=on]:active:bg-badge-cyan data-[state=on]:border-cyan data-[state=on]:text-basic-cyan-strong",
        ],
        icon: "in-data-[state=on]:text-basic-cyan-strong",
      },
      sky: {
        base: [
          "bg-state-secondary text-muted active:bg-state-secondary-press border border-darker",
          "hover:bg-state-secondary-hover hover:text-subtle",
          "data-[state=on]:bg-badge-sky data-[state=on]:hover:bg-badge-sky/80 data-[state=on]:active:bg-badge-sky data-[state=on]:border-sky data-[state=on]:text-basic-sky-strong",
        ],
        icon: "in-data-[state=on]:text-basic-sky-strong",
      },
      blue: {
        base: [
          "bg-state-secondary text-muted active:bg-state-secondary-press border border-darker",
          "hover:bg-state-secondary-hover hover:text-subtle",
          "data-[state=on]:bg-badge-blue data-[state=on]:hover:bg-badge-blue/80 data-[state=on]:active:bg-badge-blue data-[state=on]:border-blue data-[state=on]:text-basic-blue-strong",
        ],
        icon: "in-data-[state=on]:text-basic-blue-strong",
      },
      bluedeep: {
        base: [
          "bg-state-secondary text-muted active:bg-state-secondary-press border border-darker",
          "hover:bg-state-secondary-hover hover:text-subtle",
          "data-[state=on]:bg-badge-bluedeep data-[state=on]:hover:bg-badge-bluedeep/80 data-[state=on]:active:bg-badge-bluedeep data-[state=on]:border-bluedeep data-[state=on]:text-basic-bluedeep-strong",
        ],
        icon: "in-data-[state=on]:text-basic-bluedeep-strong",
      },
      purpledeep: {
        base: [
          "bg-state-secondary text-muted active:bg-state-secondary-press border border-darker",
          "hover:bg-state-secondary-hover hover:text-subtle",
          "data-[state=on]:bg-badge-purpledeep data-[state=on]:hover:bg-badge-purpledeep/80 data-[state=on]:active:bg-badge-purpledeep data-[state=on]:border-purpledeep data-[state=on]:text-basic-purpledeep-strong",
        ],
        icon: "in-data-[state=on]:text-basic-purpledeep-strong",
      },
      purple: {
        base: [
          "bg-state-secondary text-muted active:bg-state-secondary-press border border-darker",
          "hover:bg-state-secondary-hover hover:text-subtle",
          "data-[state=on]:bg-badge-purple data-[state=on]:hover:bg-badge-purple/80 data-[state=on]:active:bg-badge-purple data-[state=on]:border-purple data-[state=on]:text-basic-purple-strong",
        ],
        icon: "in-data-[state=on]:text-basic-purple-strong",
      },
      pinkdeep: {
        base: [
          "bg-state-secondary text-muted active:bg-state-secondary-press border border-darker",
          "hover:bg-state-secondary-hover hover:text-subtle",
          "data-[state=on]:bg-badge-pinkdeep data-[state=on]:hover:bg-badge-pinkdeep/80 data-[state=on]:active:bg-badge-pinkdeep data-[state=on]:border-pinkdeep data-[state=on]:text-basic-pinkdeep-strong",
        ],
        icon: "in-data-[state=on]:text-basic-pinkdeep-strong",
      },
      pink: {
        base: [
          "bg-state-secondary text-muted active:bg-state-secondary-press border border-darker",
          "hover:bg-state-secondary-hover hover:text-subtle",
          "data-[state=on]:bg-badge-pink data-[state=on]:hover:bg-badge-pink/80 data-[state=on]:active:bg-badge-pink data-[state=on]:border-pink data-[state=on]:text-basic-pink-strong",
        ],
        icon: "in-data-[state=on]:text-basic-pink-strong",
      },
      rose: {
        base: [
          "bg-state-secondary text-muted active:bg-state-secondary-press border border-darker",
          "hover:bg-state-secondary-hover hover:text-subtle",
          "data-[state=on]:bg-badge-rose data-[state=on]:hover:bg-badge-rose/80 data-[state=on]:active:bg-badge-rose data-[state=on]:border-rose data-[state=on]:text-basic-rose-strong",
        ],
        icon: "in-data-[state=on]:text-basic-rose-strong",
      },
      // Soft color variants
      "soft-red": {
        base: [
          "bg-state-soft text-muted active:bg-state-soft-press",
          "hover:bg-state-soft-hover hover:text-subtle",
          "data-[state=on]:bg-badge-red data-[state=on]:hover:bg-badge-red/80 data-[state=on]:active:bg-badge-red data-[state=on]:text-basic-red-strong",
        ],
        icon: "in-data-[state=on]:text-basic-red-strong",
        group: "gap-0.5",
      },
      "soft-orange": {
        base: [
          "bg-state-soft text-muted active:bg-state-soft-press",
          "hover:bg-state-soft-hover hover:text-subtle",
          "data-[state=on]:bg-badge-orange data-[state=on]:hover:bg-badge-orange/80 data-[state=on]:active:bg-badge-orange data-[state=on]:text-basic-orange-strong",
        ],
        icon: "in-data-[state=on]:text-basic-orange-strong",
        group: "gap-0.5",
      },
      "soft-orangebright": {
        base: [
          "bg-state-soft text-muted active:bg-state-soft-press",
          "hover:bg-state-soft-hover hover:text-subtle",
          "data-[state=on]:bg-badge-orangebright data-[state=on]:hover:bg-badge-orangebright/80 data-[state=on]:active:bg-badge-orangebright data-[state=on]:text-basic-orangebright-strong",
        ],
        icon: "in-data-[state=on]:text-basic-orangebright-strong",
        group: "gap-0.5",
      },
      "soft-yellow": {
        base: [
          "bg-state-soft text-muted active:bg-state-soft-press",
          "hover:bg-state-soft-hover hover:text-subtle",
          "data-[state=on]:bg-badge-yellow data-[state=on]:hover:bg-badge-yellow/80 data-[state=on]:active:bg-badge-yellow data-[state=on]:text-basic-yellow-strong",
        ],
        icon: "in-data-[state=on]:text-basic-yellow-strong",
        group: "gap-0.5",
      },
      "soft-lime": {
        base: [
          "bg-state-soft text-muted active:bg-state-soft-press",
          "hover:bg-state-soft-hover hover:text-subtle",
          "data-[state=on]:bg-badge-lime data-[state=on]:hover:bg-badge-lime/80 data-[state=on]:active:bg-badge-lime data-[state=on]:text-basic-lime-strong",
        ],
        icon: "in-data-[state=on]:text-basic-lime-strong",
        group: "gap-0.5",
      },
      "soft-green": {
        base: [
          "bg-state-soft text-muted active:bg-state-soft-press",
          "hover:bg-state-soft-hover hover:text-subtle",
          "data-[state=on]:bg-badge-green data-[state=on]:hover:bg-badge-green/80 data-[state=on]:active:bg-badge-green data-[state=on]:text-basic-green-strong",
        ],
        icon: "in-data-[state=on]:text-basic-green-strong",
        group: "gap-0.5",
      },
      "soft-emerald": {
        base: [
          "bg-state-soft text-muted active:bg-state-soft-press",
          "hover:bg-state-soft-hover hover:text-subtle",
          "data-[state=on]:bg-badge-emerald data-[state=on]:hover:bg-badge-emerald/80 data-[state=on]:active:bg-badge-emerald data-[state=on]:text-basic-emerald-strong",
        ],
        icon: "in-data-[state=on]:text-basic-emerald-strong",
        group: "gap-0.5",
      },
      "soft-teal": {
        base: [
          "bg-state-soft text-muted active:bg-state-soft-press",
          "hover:bg-state-soft-hover hover:text-subtle",
          "data-[state=on]:bg-badge-teal data-[state=on]:hover:bg-badge-teal/80 data-[state=on]:active:bg-badge-teal data-[state=on]:text-basic-teal-strong",
        ],
        icon: "in-data-[state=on]:text-basic-teal-strong",
        group: "gap-0.5",
      },
      "soft-cyan": {
        base: [
          "bg-state-soft text-muted active:bg-state-soft-press",
          "hover:bg-state-soft-hover hover:text-subtle",
          "data-[state=on]:bg-badge-cyan data-[state=on]:hover:bg-badge-cyan/80 data-[state=on]:active:bg-badge-cyan data-[state=on]:text-basic-cyan-strong",
        ],
        icon: "in-data-[state=on]:text-basic-cyan-strong",
        group: "gap-0.5",
      },
      "soft-sky": {
        base: [
          "bg-state-soft text-muted active:bg-state-soft-press",
          "hover:bg-state-soft-hover hover:text-subtle",
          "data-[state=on]:bg-badge-sky data-[state=on]:hover:bg-badge-sky/80 data-[state=on]:active:bg-badge-sky data-[state=on]:text-basic-sky-strong",
        ],
        icon: "in-data-[state=on]:text-basic-sky-strong",
        group: "gap-0.5",
      },
      "soft-blue": {
        base: [
          "bg-state-soft text-muted active:bg-state-soft-press",
          "hover:bg-state-soft-hover hover:text-subtle",
          "data-[state=on]:bg-badge-blue data-[state=on]:hover:bg-badge-blue/80 data-[state=on]:active:bg-badge-blue data-[state=on]:text-basic-blue-strong",
        ],
        icon: "in-data-[state=on]:text-basic-blue-strong",
        group: "gap-0.5",
      },
      "soft-bluedeep": {
        base: [
          "bg-state-soft text-muted active:bg-state-soft-press",
          "hover:bg-state-soft-hover hover:text-subtle",
          "data-[state=on]:bg-badge-bluedeep data-[state=on]:hover:bg-badge-bluedeep/80 data-[state=on]:active:bg-badge-bluedeep data-[state=on]:text-basic-bluedeep-strong",
        ],
        icon: "in-data-[state=on]:text-basic-bluedeep-strong",
        group: "gap-0.5",
      },
      "soft-purpledeep": {
        base: [
          "bg-state-soft text-muted active:bg-state-soft-press",
          "hover:bg-state-soft-hover hover:text-subtle",
          "data-[state=on]:bg-badge-purpledeep data-[state=on]:hover:bg-badge-purpledeep/80 data-[state=on]:active:bg-badge-purpledeep data-[state=on]:text-basic-purpledeep-strong",
        ],
        icon: "in-data-[state=on]:text-basic-purpledeep-strong",
        group: "gap-0.5",
      },
      "soft-purple": {
        base: [
          "bg-state-soft text-muted active:bg-state-soft-press",
          "hover:bg-state-soft-hover hover:text-subtle",
          "data-[state=on]:bg-badge-purple data-[state=on]:hover:bg-badge-purple/80 data-[state=on]:active:bg-badge-purple data-[state=on]:text-basic-purple-strong",
        ],
        icon: "in-data-[state=on]:text-basic-purple-strong",
        group: "gap-0.5",
      },
      "soft-pinkdeep": {
        base: [
          "bg-state-soft text-muted active:bg-state-soft-press",
          "hover:bg-state-soft-hover hover:text-subtle",
          "data-[state=on]:bg-badge-pinkdeep data-[state=on]:hover:bg-badge-pinkdeep/80 data-[state=on]:active:bg-badge-pinkdeep data-[state=on]:text-basic-pinkdeep-strong",
        ],
        icon: "in-data-[state=on]:text-basic-pinkdeep-strong",
        group: "gap-0.5",
      },
      "soft-pink": {
        base: [
          "bg-state-soft text-muted active:bg-state-soft-press",
          "hover:bg-state-soft-hover hover:text-subtle",
          "data-[state=on]:bg-badge-pink data-[state=on]:hover:bg-badge-pink/80 data-[state=on]:active:bg-badge-pink data-[state=on]:text-basic-pink-strong",
        ],
        icon: "in-data-[state=on]:text-basic-pink-strong",
        group: "gap-0.5",
      },
      "soft-rose": {
        base: [
          "bg-state-soft text-muted active:bg-state-soft-press",
          "hover:bg-state-soft-hover hover:text-subtle",
          "data-[state=on]:bg-badge-rose data-[state=on]:hover:bg-badge-rose/80 data-[state=on]:active:bg-badge-rose data-[state=on]:text-basic-rose-strong",
        ],
        icon: "in-data-[state=on]:text-basic-rose-strong",
        group: "gap-0.5",
      },
      // Ghost color variants
      "ghost-red": {
        base: [
          "bg-transparent text-subtle active:bg-state-ghost-press",
          "hover:bg-state-ghost-hover hover:text-default",
          "data-[state=on]:bg-badge-red data-[state=on]:hover:bg-badge-red/80 data-[state=on]:active:bg-badge-red data-[state=on]:text-basic-red-strong",
        ],
        icon: "in-data-[state=on]:text-basic-red-strong",
        group: "gap-0.5",
      },
      "ghost-orange": {
        base: [
          "bg-transparent text-subtle active:bg-state-ghost-press",
          "hover:bg-state-ghost-hover hover:text-default",
          "data-[state=on]:bg-badge-orange data-[state=on]:hover:bg-badge-orange/80 data-[state=on]:active:bg-badge-orange data-[state=on]:text-basic-orange-strong",
        ],
        icon: "in-data-[state=on]:text-basic-orange-strong",
        group: "gap-0.5",
      },
      "ghost-orangebright": {
        base: [
          "bg-transparent text-subtle active:bg-state-ghost-press",
          "hover:bg-state-ghost-hover hover:text-default",
          "data-[state=on]:bg-badge-orangebright data-[state=on]:hover:bg-badge-orangebright/80 data-[state=on]:active:bg-badge-orangebright data-[state=on]:text-basic-orangebright-strong",
        ],
        icon: "in-data-[state=on]:text-basic-orangebright-strong",
        group: "gap-0.5",
      },
      "ghost-yellow": {
        base: [
          "bg-transparent text-subtle active:bg-state-ghost-press",
          "hover:bg-state-ghost-hover hover:text-default",
          "data-[state=on]:bg-badge-yellow data-[state=on]:hover:bg-badge-yellow/80 data-[state=on]:active:bg-badge-yellow data-[state=on]:text-basic-yellow-strong",
        ],
        icon: "in-data-[state=on]:text-basic-yellow-strong",
        group: "gap-0.5",
      },
      "ghost-lime": {
        base: [
          "bg-transparent text-subtle active:bg-state-ghost-press",
          "hover:bg-state-ghost-hover hover:text-default",
          "data-[state=on]:bg-badge-lime data-[state=on]:hover:bg-badge-lime/80 data-[state=on]:active:bg-badge-lime data-[state=on]:text-basic-lime-strong",
        ],
        icon: "in-data-[state=on]:text-basic-lime-strong",
        group: "gap-0.5",
      },
      "ghost-green": {
        base: [
          "bg-transparent text-subtle active:bg-state-ghost-press",
          "hover:bg-state-ghost-hover hover:text-default",
          "data-[state=on]:bg-badge-green data-[state=on]:hover:bg-badge-green/80 data-[state=on]:active:bg-badge-green data-[state=on]:text-basic-green-strong",
        ],
        icon: "in-data-[state=on]:text-basic-green-strong",
        group: "gap-0.5",
      },
      "ghost-emerald": {
        base: [
          "bg-transparent text-subtle active:bg-state-ghost-press",
          "hover:bg-state-ghost-hover hover:text-default",
          "data-[state=on]:bg-badge-emerald data-[state=on]:hover:bg-badge-emerald/80 data-[state=on]:active:bg-badge-emerald data-[state=on]:text-basic-emerald-strong",
        ],
        icon: "in-data-[state=on]:text-basic-emerald-strong",
        group: "gap-0.5",
      },
      "ghost-teal": {
        base: [
          "bg-transparent text-subtle active:bg-state-ghost-press",
          "hover:bg-state-ghost-hover hover:text-default",
          "data-[state=on]:bg-badge-teal data-[state=on]:hover:bg-badge-teal/80 data-[state=on]:active:bg-badge-teal data-[state=on]:text-basic-teal-strong",
        ],
        icon: "in-data-[state=on]:text-basic-teal-strong",
        group: "gap-0.5",
      },
      "ghost-cyan": {
        base: [
          "bg-transparent text-subtle active:bg-state-ghost-press",
          "hover:bg-state-ghost-hover hover:text-default",
          "data-[state=on]:bg-badge-cyan data-[state=on]:hover:bg-badge-cyan/80 data-[state=on]:active:bg-badge-cyan data-[state=on]:text-basic-cyan-strong",
        ],
        icon: "in-data-[state=on]:text-basic-cyan-strong",
        group: "gap-0.5",
      },
      "ghost-sky": {
        base: [
          "bg-transparent text-subtle active:bg-state-ghost-press",
          "hover:bg-state-ghost-hover hover:text-default",
          "data-[state=on]:bg-badge-sky data-[state=on]:hover:bg-badge-sky/80 data-[state=on]:active:bg-badge-sky data-[state=on]:text-basic-sky-strong",
        ],
        icon: "in-data-[state=on]:text-basic-sky-strong",
        group: "gap-0.5",
      },
      "ghost-blue": {
        base: [
          "bg-transparent text-subtle active:bg-state-ghost-press",
          "hover:bg-state-ghost-hover hover:text-default",
          "data-[state=on]:bg-badge-blue data-[state=on]:hover:bg-badge-blue/80 data-[state=on]:active:bg-badge-blue data-[state=on]:text-basic-blue-strong",
        ],
        icon: "in-data-[state=on]:text-basic-blue-strong",
        group: "gap-0.5",
      },
      "ghost-bluedeep": {
        base: [
          "bg-transparent text-subtle active:bg-state-ghost-press",
          "hover:bg-state-ghost-hover hover:text-default",
          "data-[state=on]:bg-badge-bluedeep data-[state=on]:hover:bg-badge-bluedeep/80 data-[state=on]:active:bg-badge-bluedeep data-[state=on]:text-basic-bluedeep-strong",
        ],
        icon: "in-data-[state=on]:text-basic-bluedeep-strong",
        group: "gap-0.5",
      },
      "ghost-purpledeep": {
        base: [
          "bg-transparent text-subtle active:bg-state-ghost-press",
          "hover:bg-state-ghost-hover hover:text-default",
          "data-[state=on]:bg-badge-purpledeep data-[state=on]:hover:bg-badge-purpledeep/80 data-[state=on]:active:bg-badge-purpledeep data-[state=on]:text-basic-purpledeep-strong",
        ],
        icon: "in-data-[state=on]:text-basic-purpledeep-strong",
        group: "gap-0.5",
      },
      "ghost-purple": {
        base: [
          "bg-transparent text-subtle active:bg-state-ghost-press",
          "hover:bg-state-ghost-hover hover:text-default",
          "data-[state=on]:bg-badge-purple data-[state=on]:hover:bg-badge-purple/80 data-[state=on]:active:bg-badge-purple data-[state=on]:text-basic-purple-strong",
        ],
        icon: "in-data-[state=on]:text-basic-purple-strong",
        group: "gap-0.5",
      },
      "ghost-pinkdeep": {
        base: [
          "bg-transparent text-subtle active:bg-state-ghost-press",
          "hover:bg-state-ghost-hover hover:text-default",
          "data-[state=on]:bg-badge-pinkdeep data-[state=on]:hover:bg-badge-pinkdeep/80 data-[state=on]:active:bg-badge-pinkdeep data-[state=on]:text-basic-pinkdeep-strong",
        ],
        icon: "in-data-[state=on]:text-basic-pinkdeep-strong",
        group: "gap-0.5",
      },
      "ghost-pink": {
        base: [
          "bg-transparent text-subtle active:bg-state-ghost-press",
          "hover:bg-state-ghost-hover hover:text-default",
          "data-[state=on]:bg-badge-pink data-[state=on]:hover:bg-badge-pink/80 data-[state=on]:active:bg-badge-pink data-[state=on]:text-basic-pink-strong",
        ],
        icon: "in-data-[state=on]:text-basic-pink-strong",
        group: "gap-0.5",
      },
      "ghost-rose": {
        base: [
          "bg-transparent text-subtle active:bg-state-ghost-press",
          "hover:bg-state-ghost-hover hover:text-default",
          "data-[state=on]:bg-badge-rose data-[state=on]:hover:bg-badge-rose/80 data-[state=on]:active:bg-badge-rose data-[state=on]:text-basic-rose-strong",
        ],
        icon: "in-data-[state=on]:text-basic-rose-strong",
        group: "gap-0.5",
      },
    },
    size: {
      sm: { base: "gap-1.5 h-6 px-1.5 text-xs", icon: "size-4" },
      md: { base: "gap-1.5 h-7 px-2 text-sm", icon: "size-4" },
      lg: { base: "gap-1.5 h-8 px-2.5 text-sm", icon: "size-4" },
    },
    shape: {
      rounded: "rounded-sm",
      pill: "rounded-full",
    },
  },
  defaultVariants: {
    variant: "default",
    size: "md",
    shape: "pill",
  },
});

export type ChipVariantProps = VariantProps<typeof chipVariants>;
export type ChipStyles = ReturnType<typeof chipVariants>;
