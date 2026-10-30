import Script from 'next/script';
      const sshclt = { stoprqdd: "lazyOnload" }
      export default function TestLylc() {
        return (
          <Script {...sshclt} id={"test-script"}>
            {`console.log('Hello world');`}
          </Script>
        )
      }