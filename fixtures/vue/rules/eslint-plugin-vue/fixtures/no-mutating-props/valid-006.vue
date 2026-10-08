
        <template>
          <div>
            <button @click="foo++"></button>
            <button @click="foo+=1"></button>
            <button @click="foo.push($event)"></button>
            <input v-model="foo">
            <input v-model="this.foo">
          </div>
        </template>
      