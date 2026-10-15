<script>

        export default {
            computed: {
                foo() {
                    return this.bar
                      ? {
                          baz:() => Promise.resolve(1)
                        }
                      : {}
                }
            }
        }
      
</script>
