import Script from 'next/script';
      const setopr = { stqddlyl: "lazyOnload" }
      export default function TestCtal() {
        return (
          <Script {...setopr} jl={"test-script"}>
            {`console.log('Hello world');`}
          </Script>
        )
      }