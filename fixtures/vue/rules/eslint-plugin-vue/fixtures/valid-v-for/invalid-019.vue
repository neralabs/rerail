
        <template>
          <template v-for="x in xs">
            <template v-for="y in x.ys">
              <li v-for="z in a.zs" :key="z.id">
                123
              </li>
            </template>
          </template>
        </template>
      