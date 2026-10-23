
        <script>
          export default {
            setup(props) {
                props.prop1.text = '2'
                props.prop1.count ++
                props.prop1.list.push(1)
                props.prop1.parent.text = '2'
            }
          }
        </script>
      