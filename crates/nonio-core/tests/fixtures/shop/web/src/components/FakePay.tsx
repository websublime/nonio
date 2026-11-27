const axios = { post: (url: string) => url };

export function FakePay() {
  const pay = () => axios.post('/api/invoices/1/pay');
  return <button onClick={pay}>Pay</button>;
}
