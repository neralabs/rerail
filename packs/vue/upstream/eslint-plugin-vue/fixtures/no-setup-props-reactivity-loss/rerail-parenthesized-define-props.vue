<script setup>
const props = (defineProps({ a: String }))
const { a } = props
const q = ((defineProps({ b: String })))
const { b } = q
const w = (withDefaults(defineProps({ c: String }), {}))
const { c } = w
</script>
