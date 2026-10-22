
        <template>
          <div>
            <div v-if="prop1 = [1, 2]"></div>
            <div v-if="prop2++"></div>
            <div v-text="prop3.shift()"></div>
            <div v-text="prop4.slice(0).shift()"></div>
            <div v-if="this.prop5 = [1, 2] && this.someProp"></div>
            <div v-if="this.prop6++ && this.someProp < 10"></div>
            <div v-text="this.prop7.shift()"></div>
            <div v-text="this.prop8.slice(0).shift()"></div>
            <div v-if="delete prop9.a"></div>
            <div v-if="delete this.prop10"></div>
          </div>
        </template>
        <script>
          export default {
            props: ['prop1', 'prop2', 'prop3', 'prop4', 'prop5', 'prop6', 'prop7', 'prop8', 'prop9', 'prop10'],
            method: {
              deleteProp() {
                delete this.prop9.a
                delete this.prop10
              }
            }
          }
        </script>
      