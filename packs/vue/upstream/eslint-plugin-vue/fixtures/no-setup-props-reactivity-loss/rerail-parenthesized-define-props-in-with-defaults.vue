<script setup>
const props = withDefaults((defineProps({ c: String })), {})
const { c } = props
const d = defineProps /* x */ ({ d: String })
const { d: dd } = d
</script>
