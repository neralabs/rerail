
      <template>
        <input ref="a" />
        <input ref="b" />
        <input ref="c" />
        <input ref="d" />
        <input ref="e" />
        <input ref="f" />
      </template>
      <script>
      export default {
        mounted() {
          this.$refs.a;
          const {b} = this.$refs;
          ({c} = this.$refs);
          $refs.d;
          const {e} = $refs;
          ({f} = $refs);
        }
      }
      </script>
      