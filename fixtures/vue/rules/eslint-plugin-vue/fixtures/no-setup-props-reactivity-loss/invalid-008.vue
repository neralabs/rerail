
      <script>
      export default {
        setup(p) {
          const x = p.foo
          const y = p?.bar
          const z = (p?.baz).qux

          const xc = p?.foo?.()
          const yc = (p?.bar)?.()
          const zc = (p?.baz.qux)?.()
        }
      }
      </script>
      