<script lang="ts">
import { defineComponent } from 'vue'
import { formatTotal } from '../utils/format'

export function load() {
  return 0
}

export default defineComponent({
  data() {
    return { n: load() }
  },
  methods: {
    show() {
      return formatTotal(this.n)
    },
  },
})
</script>

<template>
  <span>{{ show() }}</span>
</template>
