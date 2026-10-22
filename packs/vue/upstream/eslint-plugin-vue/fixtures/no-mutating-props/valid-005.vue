
        <template>
          <div v-if="prop">
            <div v-for="prop in data">
              <MyComp @click="prop.foo++"></MyComp>
              <input v-model="prop">
            </div>
            <input v-model="prop()">
            <input v-model="foo">
            <input @click="prop().foo++">
            <input v-model="foo[this]">
            <input v-model="foo[this.prop]">
            <input v-model="this">
            <MyComp @click="bar = {prop: foo++}"></MyComp>
          </div>
        </template>
        <script>
          export default {
            props: ['prop'],
            methods: {
              onKeydown() {
                const vm = this
                foo.prop = 1
                vm()()()
                vm.prop()()
                prop++
                prop = 1
                const bar = {prop: foo}
                prop[this] ++
              }
            }
          }
        </script>
      