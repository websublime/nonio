<script setup lang="ts">
import { computed, onMounted } from 'vue'
import OrderRow from './OrderRow.vue'
import { useOrders } from '../composables/useOrders'
import { formatTotal } from '../utils/format'
import { http } from '../api/client'

const props = defineProps<{ title: string }>()
const { orders, load } = useOrders()
const total = computed(() => orders.value.reduce((s, o) => s + o.total, 0))

function refresh() {
  load()
  http.get('/api/orders/refresh')
}

function pick(id: number) {
  return id
}

onMounted(() => refresh())
</script>

<template>
  <section>
    <h1>{{ props.title }}</h1>
    <p>{{ formatTotal(total) }}</p>
    <button @click="refresh">Refresh</button>
    <OrderRow v-for="o in orders" :key="o.id" :order="o" @select="pick(o.id)" @load="o.load()" />
  </section>
</template>
