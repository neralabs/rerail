<script>
app.component('test', {
        computed: {
          fooBar() {
            this.$set(this, 'foo', 'lorem');
            Vue.set(this, 'bar', 'ipsum');
            return this.foo + ' ' + this.bar
          },
        }
      })
</script>
