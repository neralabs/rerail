import * as React from "react";
import { cva, type VariantProps } from "class-variance-authority";

import { cn } from "@/lib/utils";

/*
 * Typography primitive.
 *
 *   <Text variant="headline">Good evening, User</Text>
 *   <Text variant="body-strong" weight="semibold">Emphasized</Text>
 *   <Text variant="caption" role="alert">{error}</Text>
 *
 * One primitive, two axes: `variant` owns size / line-height / tracking /
 * default-color / (mono / uppercase / tabular where semantic). `weight`
 * owns font-weight and is deliberately minimal — regular / medium / semibold.
 *
 * Each variant advertises its default weight below. Callers override the
 * default with the `weight` prop rather than `font-*` classes on `className`.
 *
 * variant → element + type spec + default weight:
 *
 * | variant     | Element | Size / LH / Track   | Default | Notes           |
 * | ----------- | ------- | ------------------- | ------- | --------------- |
 * | headline    | h1      | 24 / 32 · -0.01em   | semi    | largest tier    |
 * | title       | h2      | 20 / 28 · -0.005em  | semi    | section titles  |
 * | subtitle    | h3      | 16 / 24             | medium  | card titles     |
 * | body-lg     | p       | 16 / 24             | regular | emphasis body   |
 * | body        | p       | 14 / 20             | regular | default body    |
 * | body-strong | span    | 14 / 20             | medium  | labels / chips  |
 * | body-muted  | p       | 14 / 20 muted       | regular | supporting copy |
 * | caption     | span    | 12 / 16 muted       | regular | secondary       |
 * | overline    | span    | 10 / 12 · +0.08em · UPPER · muted | semi | eyebrow labels |
 * | meta        | span    | 10 / 12 · subtle    | regular | timestamps/ids  |
 * | metric      | span    | 20 / 24 · tabular   | semi    | primary metric  |
 * | metric-sm   | span    | 14 / 20 · tabular   | medium  | table numeric   |
 * | code        | code    | 14 · mono           | inherit | inline code     |
 * | code-block  | code    | 13 / 20 · mono      | regular | fenced blocks   |
 *
 * `as` is for accessibility overrides only (a heading level chosen
 * independently of the visual tier). Do not use it to defeat a variant's
 * default element — pick a different variant instead.
 *
 * `textVariants` is exported for the rare node that cannot be a `<Text>`
 * (a `<label>`, a `<summary>`, a tag chosen at runtime). It is the only
 * sanctioned way to apply a variant's type outside this file.
 */

const textVariants = cva("", {
  variants: {
    variant: {
      headline: "text-2xl leading-8 tracking-[-0.01em] text-foreground",
      title: "text-xl leading-7 tracking-[-0.005em] text-foreground",
      subtitle: "text-base leading-6 text-foreground",
      "body-lg": "text-base leading-6 text-foreground",
      body: "text-sm leading-5 text-foreground",
      "body-strong": "text-sm leading-5 text-foreground",
      "body-muted": "text-sm leading-5 text-muted-foreground",
      caption: "text-xs leading-4 text-muted-foreground",
      overline: "text-2xs leading-3 tracking-[0.08em] uppercase text-muted-foreground",
      meta: "text-2xs leading-3 text-subtle-foreground",
      metric: "text-xl leading-6 tabular-nums text-foreground",
      "metric-sm": "text-sm leading-5 tabular-nums text-foreground",
      code: "font-mono text-sm",
      "code-block": "font-mono text-[0.8125rem] leading-5",
    },
    weight: {
      regular: "font-normal",
      medium: "font-medium",
      semibold: "font-semibold",
    },
  },
  defaultVariants: {
    variant: "body",
  },
});

export type TextVariant = NonNullable<VariantProps<typeof textVariants>["variant"]>;
export type TextWeight = NonNullable<VariantProps<typeof textVariants>["weight"]>;

/**
 * Per-variant default weight. Applied when the caller omits `weight`.
 * `code` inherits its weight from surrounding prose (null).
 */
const defaultWeightForVariant: Record<TextVariant, TextWeight | null> = {
  headline: "semibold",
  title: "semibold",
  subtitle: "medium",
  "body-lg": "regular",
  body: "regular",
  "body-strong": "medium",
  "body-muted": "regular",
  caption: "regular",
  overline: "semibold",
  meta: "regular",
  metric: "semibold",
  "metric-sm": "medium",
  code: null,
  "code-block": "regular",
};

type TextElement =
  | "h1"
  | "h2"
  | "h3"
  | "h4"
  | "h5"
  | "h6"
  | "p"
  | "span"
  | "div"
  | "code"
  | "label";

const variantToElement: Record<TextVariant, TextElement> = {
  headline: "h1",
  title: "h2",
  subtitle: "h3",
  "body-lg": "p",
  body: "p",
  "body-strong": "span",
  "body-muted": "p",
  caption: "span",
  overline: "span",
  meta: "span",
  metric: "span",
  "metric-sm": "span",
  code: "code",
  "code-block": "code",
};

type TextProps = {
  variant: TextVariant;
  weight?: TextWeight;
  as?: TextElement;
  htmlFor?: string;
} & React.HTMLAttributes<HTMLElement>;

const Text = React.forwardRef<HTMLElement, TextProps>(function Text(
  { variant, weight, as, className, htmlFor, children, ...rest },
  ref,
) {
  const Tag = as ?? variantToElement[variant];
  const effectiveWeight = weight ?? defaultWeightForVariant[variant] ?? undefined;
  return (
    <Tag
      ref={ref as never}
      htmlFor={htmlFor}
      className={cn(textVariants({ variant, weight: effectiveWeight }), className)}
      {...rest}
    >
      {children}
    </Tag>
  );
});

export { Text, textVariants };
