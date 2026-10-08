
      <template>
        <!-- ✓ GOOD -->
        <input ref="foo" />
      </template>
      <script>
      export default {
        mounted() {
          this.$refs.foo.value = 'foo'
        }
      }
      </script>
      