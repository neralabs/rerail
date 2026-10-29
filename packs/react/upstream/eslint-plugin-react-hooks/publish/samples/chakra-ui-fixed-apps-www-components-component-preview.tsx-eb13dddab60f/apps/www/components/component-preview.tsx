import { Box } from "@chakra-ui/react"
import dynamic from "next/dynamic"
import { type ComponentType, createElement } from "react"
import { CodeHighlight } from "./code-highlight"

interface Props {
  name: string
}

function formatComponentName(name: string) {
  return name
    .split("-")
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join("")
}

// Each example's loader is created once and reused, so re-rendering a preview keeps its state.
const previews = new Map<string, ComponentType>()

function getPreview(name: string): ComponentType {
  let preview = previews.get(name)
  if (!preview) {
    const componentName = formatComponentName(name)
    preview = dynamic(() =>
      import(`../../compositions/src/examples/${name}`).then(
        (mod) => mod[componentName],
      ),
    )
    previews.set(name, preview)
  }
  return preview
}

export const ComponentPreview = (props: Props) => {
  const { name } = props

  return (
    <Box borderWidth="1px" rounded="lg" overflow="hidden" divideY="1px">
      <Box padding="10">
        {createElement(getPreview(name))}
      </Box>
      <Box
        css={{
          position: "relative",
          "& pre": {
            px: "10",
            py: "6",
            maxHeight: "400px",
            overflow: "auto",
          },
          "& code": {
            fontSize: "0.82rem",
            "& *": {
              fontStyle: "normal!",
            },
          },
          _dark: {
            "& :is(.shiki, .shiki span)": {
              color: "var(--shiki-dark)!",
              bg: "var(--shiki-dark-bg)!",
            },
          },
        }}
      >
        <CodeHighlight name={name} />
      </Box>
    </Box>
  )
}
