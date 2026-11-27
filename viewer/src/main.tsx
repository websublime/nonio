import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import '@fontsource-variable/instrument-sans';
import '@xyflow/react/dist/style.css';
import './styles.css';
import './wireframe.css';
import './code.css';
import './journey.css';
import { App } from './app';

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
