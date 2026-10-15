
        <template>
          <div>
            <input v-model="prop1.text">
            <input v-model="prop2">
            <input v-model="this.prop3.text">
            <input v-model="this.prop4">
          </div>
        </template>
        <script>
          export default {
            props: ['prop5', 'prop6', 'prop7', 'prop8']
          }
        </script>
      