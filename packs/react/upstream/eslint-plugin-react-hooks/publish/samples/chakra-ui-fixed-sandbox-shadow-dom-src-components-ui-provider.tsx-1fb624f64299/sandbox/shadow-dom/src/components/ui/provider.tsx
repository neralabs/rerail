"use client"

import {
  ChakraProvider,
  EnvironmentProvider,
  createSystem,
  defaultConfig,
  defineConfig,
} from "@chakra-ui/react"
import createCache from "@emotion/cache"
import { CacheProvider } from "@emotion/react"
import { ThemeProvider, type ThemeProviderProps } from "next-themes"
import { useMemo, useState } from "react"
import root from "react-shadow/emotion"

const varRoot = ":host"

const config = defineConfig({
  cssVarsRoot: varRoot,
  conditions: {
    light: `${varRoot}, .light`,
  },
  preflight: { scope: varRoot },
  globalCss: {
    [varRoot]: defaultConfig.globalCss!.html,
  },
})

const system = createSystem(defaultConfig, config)

export function Provider(props: ThemeProviderProps) {
  const [shadow, setShadow] = useState<HTMLElement | null>(null)
  const cache = useMemo(() => {
    if (!shadow?.shadowRoot) return null
    return createCache({
      key: "root",
      container: shadow.shadowRoot,
    })
  }, [shadow])

  return (
    <root.div ref={setShadow}>
      {shadow && cache && (
        <EnvironmentProvider value={() => shadow.shadowRoot ?? document}>
          <CacheProvider value={cache}>
            <ChakraProvider value={system}>
              <ThemeProvider
                disableTransitionOnChange
                attribute="class"
                {...props}
              />
            </ChakraProvider>
          </CacheProvider>
        </EnvironmentProvider>
      )}
    </root.div>
  )
}
