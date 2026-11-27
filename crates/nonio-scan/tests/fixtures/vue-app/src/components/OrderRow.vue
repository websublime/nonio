<script setup lang="ts">
const props = defineProps<{ order: { id: number; total: number } }>()
const emit = defineEmits<{ select: [] }>()
</script>

<template>
  <li @click="emit('select')">{{ props.order.id }}</li>
</template>
