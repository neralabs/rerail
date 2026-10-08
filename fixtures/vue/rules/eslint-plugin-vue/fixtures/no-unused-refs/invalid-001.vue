
      <template>
        <!-- ✓ GOOD -->
        <input ref="foo" />

        <!-- ✗ BAD (bar is not used) -->
        <input ref="bar" />
      </template>
      <script>
      export default {
        mounted() {
          this.$refs.foo.value = 'foo'
        }
      }
      </script>
      