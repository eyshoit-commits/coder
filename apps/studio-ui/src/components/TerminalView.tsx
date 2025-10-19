import { useEffect, useRef, useState } from 'react';
import { Terminal } from '@xterm/xterm';
import '@xterm/xterm/css/xterm.css';

export function TerminalView() {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const [status, setStatus] = useState('connecting');

  useEffect(() => {
    if (!containerRef.current) {
      return;
    }
    const terminal = new Terminal({
      convertEol: true,
      theme: {
        background: '#05070f',
        foreground: '#f8fafc'
      },
      fontFamily: 'JetBrains Mono, monospace',
      fontSize: 13
    });
    terminal.open(containerRef.current);
    terminal.writeln('\x1b[36mCyberDevStudio interactive terminal\x1b[0m');
    const resolvedUrl = (import.meta.env.VITE_TERMINAL_URL as string | undefined) ??
      `${window.location.protocol === 'https:' ? 'wss' : 'ws'}://${window.location.host}/ws/terminal`;
    const socket = new WebSocket(resolvedUrl);

    socket.addEventListener('open', () => {
      setStatus('connected');
      terminal.writeln('\r\nConnected to sandbox runtime.');
    });
    socket.addEventListener('close', () => {
      setStatus('disconnected');
      terminal.writeln('\r\nDisconnected from runtime.');
    });
    socket.addEventListener('message', (event) => {
      terminal.write(event.data as string);
    });

    const writeToSocket = (data: string) => {
      if (socket.readyState === WebSocket.OPEN) {
        socket.send(data);
      }
    };

    const disposable = terminal.onData((data) => {
      writeToSocket(data);
    });

    return () => {
      disposable.dispose();
      socket.close();
      terminal.dispose();
    };
  }, []);

  return (
    <div className="flex h-full flex-col bg-[color:var(--bg-primary)]/80">
      <div className="border-b border-slate-800/60 bg-[color:var(--panel)] px-4 py-2 text-xs uppercase tracking-wide text-[color:var(--text-secondary)]">
        Terminal status: <span className="text-[color:var(--accent-1)]">{status}</span>
      </div>
      <div ref={containerRef} className="flex-1 bg-[color:var(--bg-primary)]/60"></div>
    </div>
  );
}
