import axios from 'axios';

export function PayButton({ invoice }: { invoice: number }) {
  const pay = () => axios.post(`/api/invoices/${invoice}/pay`);
  return <button onClick={pay}>Pay</button>;
}
