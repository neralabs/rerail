<script>
import { defineComponent } from 'elsewhere'
defineComponent({
  props: ['value'],
  methods: { m() { this.value = 1 } }
})
</script>
