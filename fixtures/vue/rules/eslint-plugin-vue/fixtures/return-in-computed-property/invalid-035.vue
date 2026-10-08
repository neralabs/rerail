<script>

        export default {
          computed: {
            foo () {
              switch (this.type) {
                case 'a':
                  return 1
                case 'b':
                  const fn = () => { return 2 }
                  fn()
              }
            }
          }
        }
      
</script>
