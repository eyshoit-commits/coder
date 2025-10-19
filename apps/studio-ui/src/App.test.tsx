import { describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import App from './App';

function renderApp(initialEntries: string[] = ['/']) {
  render(
    <MemoryRouter initialEntries={initialEntries}>
      <App />
    </MemoryRouter>
  );
}

describe('App', () => {
  it('renders the editor tab by default', () => {
    renderApp();
    expect(screen.getByText('Save')).toBeInTheDocument();
  });

  it('allows navigation to LLM tab', async () => {
    renderApp(['/llm']);
    expect(await screen.findByText(/Generate/)).toBeInTheDocument();
  });
});
