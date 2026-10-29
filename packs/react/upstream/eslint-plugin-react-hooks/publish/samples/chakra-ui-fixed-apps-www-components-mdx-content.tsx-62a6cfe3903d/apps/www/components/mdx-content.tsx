import { Kbd } from "@chakra-ui/react"
import { AspectRatioTokenDoc } from "compositions/lib/aspect-ratio-token-toc"
import { BorderRadiusTokenDoc } from "compositions/lib/border-radius-token-doc"
import { BreakpointDoc } from "compositions/lib/breakpoint-doc"
import {
  ColorSemanticTokenDoc,
  ColorTokenDoc,
} from "compositions/lib/color-token-doc"
import { DurationTokenDoc, KeyframeDoc } from "compositions/lib/keyframe-doc"
import { ShadowTokenDoc } from "compositions/lib/shadow-token-doc"
import { SizesTokenDoc } from "compositions/lib/sizes-token-doc"
import { SpacingTokenDoc } from "compositions/lib/spacing-token-doc"
import * as TypographyDocs from "compositions/lib/typography-token-doc"
import { ZIndexTokenDoc } from "compositions/lib/z-index-token-doc"
import { type ComponentType, createElement } from "react"
import * as runtime from "react/jsx-runtime"
import { Example, ExamplePreview, ExampleTabs } from "./example"
import { Anchor } from "./mdx/anchor"
import { Blockquote } from "./mdx/blockquote"
import { Callout } from "./mdx/callout"
import { Card, CardGroup } from "./mdx/card"
import { Code, Pre } from "./mdx/code"
import { CodeBlock } from "./mdx/code-block"
import { CodeGroup } from "./mdx/code-group"
import { ComponentGrid } from "./mdx/component-grid"
import { H1, H2, H3, H4 } from "./mdx/heading"
import { Img } from "./mdx/image"
import { Li, Ol, Ul } from "./mdx/list"
import { PropTable } from "./mdx/prop-table"
import { ResourceCard } from "./mdx/resource-card"
import { Steps } from "./mdx/steps"
import { Table } from "./mdx/table"
import { P, Strong } from "./mdx/text"

const sharedComponents = {
  a: Anchor,
  blockquote: Blockquote,
  img: Img,
  p: P,
  strong: Strong,
  h1: H1,
  h2: H2,
  h3: H3,
  h4: H4,
  kbd: Kbd,
  pre: Pre,
  code: Code,
  ol: Ol,
  ul: Ul,
  li: Li,
  table: Table,
  steps: Steps,
  callout: Callout,
  "code-group": CodeGroup,
  Example: Example,
  ExampleTabs: ExampleTabs,
  ExamplePreview: ExamplePreview,
  card: Card,
  "card-group": CardGroup,
  Card,
  PropTable,
  ComponentGrid,
  ResourceCard: ResourceCard,
  "code-block": CodeBlock,
  ColorTokenDoc,
  ColorSemanticTokenDoc,
  ShadowTokenDoc,
  BorderRadiusTokenDoc,
  ...TypographyDocs,
  SpacingTokenDoc,
  KeyframeDoc,
  BreakpointDoc,
  DurationTokenDoc,
  AspectRatioTokenDoc,
  SizesTokenDoc,
  ZIndexTokenDoc,
}

type MDXComponent = ComponentType<{ components: Record<string, React.ComponentType> }>

// Each compiled document becomes one component, created once and reused, so re-rendering keeps its state.
const mdxComponents = new Map<string, MDXComponent>()

const getMDXComponent = (code: string): MDXComponent => {
  let component = mdxComponents.get(code)
  if (!component) {
    const fn = new Function(code)
    component = fn({ ...runtime }).default as MDXComponent
    mdxComponents.set(code, component)
  }
  return component
}

interface MDXProps {
  code: string
  components?: Record<string, React.ComponentType>
}

export const MDXContent = (props: MDXProps) => {
  const { code, components = {} } = props
  return createElement(getMDXComponent(code), {
    components: { ...sharedComponents, ...components },
  })
}
