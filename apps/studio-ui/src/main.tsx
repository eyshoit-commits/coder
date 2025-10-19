import React from 'react';
import ReactDOM from 'react-dom/client';
import { BrowserRouter } from 'react-router-dom';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import App from './App';
import './styles/tailwind.css';
import './themes/NeonCyberNight.css';
import './themes/SerialSteel.css';

const queryClient = new QueryClient();

const rootElement = document.getElementById('root');
if (!rootElement) {
  throw new Error('Unable to find root element for Studio UI bootstrap.');
}

const body = document.body;
const storedTheme = localStorage.getItem('cyberdevstudio.theme');
if (storedTheme === 'theme-neon' || storedTheme === 'theme-steel') {
  body.classList.add(storedTheme);
} else if (!body.classList.contains('theme-neon') && !body.classList.contains('theme-steel')) {
  body.classList.add('theme-neon');
}

ReactDOM.createRoot(rootElement).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <BrowserRouter>
        <App />
      </BrowserRouter>
    </QueryClientProvider>
  </React.StrictMode>
);
