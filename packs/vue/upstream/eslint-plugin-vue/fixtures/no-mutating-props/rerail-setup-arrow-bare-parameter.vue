<script>
export default {
  props: { count: Number },
  setup: props => {
    props.count++
  }
}
</script>
