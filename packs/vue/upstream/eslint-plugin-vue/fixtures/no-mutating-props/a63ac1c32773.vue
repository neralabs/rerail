
        <template>
          <div>
            <input v-model="prop1.text">
            <input v-model="this.prop2.text">
            <button @click="prop3.text = '1'"></button>
            <button @click="prop3.count++"></button>
            <button @click="prop3.list.push(1)"></button>
            <button @click="prop3.parent.text = '2'"></button>
            <button @click="delete prop3.parent.text"></button>
          </div>
        </template>
        <script>
          export default {
            props: ['prop1', 'prop2', 'prop3'],
            methods: {
                onKeydown() {
                  this.prop3.text = '2'
                  this.prop3.count ++
                  this.prop3.list.push(1)
                  this.prop3.parent.text = '2'
                  delete this.prop3.parent.text
                }
            }
          }
        </script>
      