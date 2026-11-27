'use client';

export function StockBadge() {
  const check = () => fetch('/api/stock');
  return <button onClick={check}>Stock</button>;
}
