<script>
const { defineComponent } = require('elsewhere')
defineComponent({
  props: ['value'],
  methods: { m() { this.value = 1 } }
})
</script>
