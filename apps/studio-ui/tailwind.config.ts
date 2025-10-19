import type { Config } from 'tailwindcss';

const config: Config = {
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  theme: {
    extend: {
      colors: {
        neon: {
          primary: '#0ff0fc',
          secondary: '#f500ff',
          accent: '#00ff85'
        },
        steel: {
          primary: '#1f2933',
          secondary: '#cbd2d9',
          accent: '#486581'
        }
      },
      fontFamily: {
        mono: ['"JetBrains Mono"', 'monospace'],
        sans: ['"Inter"', 'sans-serif']
      },
      boxShadow: {
        glow: '0 0 20px rgba(15, 240, 252, 0.3)',
        panel: '0 20px 45px rgba(15, 23, 42, 0.35)'
      }
    }
  },
  plugins: []
};

export default config;
