<script>
import { defineComponent } from 'vue'
defineComponent({
  props: ['value'],
  methods: { m() { this.value = 1 } }
})
</script>
