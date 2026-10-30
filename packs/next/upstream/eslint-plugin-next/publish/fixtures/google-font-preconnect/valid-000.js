export const Test = () => (
        <div>
          <link rel="preconnect" href="https://fonts.gstatic.com"/>
          <link
            href={process.env.NEXT_PUBLIC_CANONICAL_URL}
            rel="canonical"
          />
          <link
            href={new URL("../public/favicon.ico", import.meta.url).toString()}
            rel="icon"
          />
        </div>
      )
    