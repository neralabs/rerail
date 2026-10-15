<script>
function defineComponent(options) { return options }
defineComponent({
  props: ['value'],
  methods: { m() { this.value = 1 } }
})
</script>
