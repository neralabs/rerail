import * as React from "react"
import { cva, type VariantProps } from "class-variance-authority"
import { Slot } from "radix-ui"

import { cn } from "@/lib/utils"

const buttonVariants = cva(
  "group/button inline-flex shrink-0 items-center justify-center border border-transparent bg-clip-padding text-sm font-medium whitespace-nowrap transition-all outline-none select-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 active:not-aria-[haspopup]:translate-y-px disabled:pointer-events-none disabled:opacity-50 aria-invalid:border-destructive aria-invalid:ring-3 aria-invalid:ring-destructive/20 dark:aria-invalid:border-destructive/50 dark:aria-invalid:ring-destructive/40 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4",
  {
    variants: {
      // Variants name the role a button plays, not how it looks. Role names stay
      // bounded as the palette grows; appearance names ("indigo-ghost",
      // "success-soft") multiply with every colour. Only the primary action is
      // brand-coloured — everything below it recedes so it can't compete.
      variant: {
        primary: "bg-primary text-primary-foreground hover:bg-primary/80",
        secondary:
          "bg-brand-subtle text-brand-foreground hover:bg-brand-subtle-hover aria-expanded:bg-brand-subtle",
        outline:
          "border-border-strong bg-background text-foreground hover:bg-subtle aria-expanded:bg-subtle",
        ghost: "text-foreground hover:bg-subtle aria-expanded:bg-subtle",
        destructive:
          "bg-destructive/10 text-destructive hover:bg-destructive/20 focus-visible:border-destructive/40 focus-visible:ring-destructive/20 dark:bg-destructive/20 dark:hover:bg-destructive/30 dark:focus-visible:ring-destructive/40",
        // Text-link Button: bare interactive text with the shared focus ring,
        // no surface treatment. Pair with `size="inline"` for zero padding / no
        // fixed height. Color/size/weight come from the consumer's `className`.
        link: "border-transparent bg-transparent text-brand-foreground underline-offset-2 hover:underline",
      },
      size: {
        default:
          "h-8 gap-1.5 px-4 has-data-[icon=inline-end]:pr-3 has-data-[icon=inline-start]:pl-3",
        xs: "h-6 gap-1 px-3 text-xs has-data-[icon=inline-end]:pr-2 has-data-[icon=inline-start]:pl-2 [&_svg:not([class*='size-'])]:size-3",
        sm: "h-7 gap-1 px-3.5 text-[0.8rem] has-data-[icon=inline-end]:pr-2.5 has-data-[icon=inline-start]:pl-2.5 [&_svg:not([class*='size-'])]:size-3.5",
        lg: "h-9 gap-1.5 px-4 has-data-[icon=inline-end]:pr-3 has-data-[icon=inline-start]:pl-3",
        icon: "size-8",
        "icon-xs":
          "size-6 [&_svg:not([class*='size-'])]:size-3",
        "icon-sm":
          "size-7",
        "icon-lg": "size-9",
        // Height/padding-free size for inline links inside prose or dense
        // toolbars. Pair with `variant="link"`.
        inline: "h-auto gap-1 p-0",
      },
      radius: {
        none: "rounded-none",
        small: "rounded-md",
        medium: "rounded-lg",
        large: "rounded-xl",
        full: "rounded-full",
      },
    },
    defaultVariants: {
      variant: "primary",
      size: "default",
      radius: "full",
    },
  }
)

function Button({
  className,
  variant = "primary",
  size = "default",
  radius = "full",
  asChild = false,
  ...props
}: React.ComponentProps<"button"> &
  VariantProps<typeof buttonVariants> & {
    asChild?: boolean
  }) {
  const Comp = asChild ? Slot.Root : "button"

  return (
    <Comp
      data-slot="button"
      data-variant={variant}
      data-size={size}
      data-radius={radius}
      className={cn(buttonVariants({ variant, size, radius, className }))}
      {...props}
    />
  )
}

export { Button, buttonVariants }
