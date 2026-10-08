
        <script>
          export default {
            setup({a,b,c, d: [e, , f]}) {
              a.foo ++
              b.foo = 1
              c.push(1)

              c.x.push(1)
              delete c.y
              e.foo++
              f.foo++
            }
          }
        </script>
      