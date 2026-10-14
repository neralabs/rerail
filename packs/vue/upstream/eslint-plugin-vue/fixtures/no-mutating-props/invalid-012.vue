
        <script>
          export default {
            setup({a: foo, b: [...bar], c: baz = 1}) {
              foo.x ++
              delete foo.y
              bar.x = 1
              baz.push(1)
            }
          }
        </script>
      