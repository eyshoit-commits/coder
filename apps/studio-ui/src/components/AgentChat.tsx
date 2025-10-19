import { FormEvent, useEffect, useRef, useState } from 'react';
import { io, Socket } from 'socket.io-client';
import { useStudioContext } from '../hooks/useStudioContext';

interface ChatMessage {
  id: string;
  role: 'user' | 'agent' | 'system';
  agent?: string;
  content: string;
  createdAt: string;
}

const agentOptions = [
  { id: 'code', label: 'CodeAgent' },
  { id: 'test', label: 'TestAgent' },
  { id: 'design', label: 'DesignAgent' },
  { id: 'debug', label: 'DebugAgent' },
  { id: 'security', label: 'SecurityAgent' },
  { id: 'doc', label: 'DocAgent' }
];

export function AgentChat() {
  const { rpc } = useStudioContext();
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [agent, setAgent] = useState(agentOptions[0].id);
  const [input, setInput] = useState('');
  const [isStreaming, setIsStreaming] = useState(false);
  const socketRef = useRef<Socket | null>(null);
  const viewportRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const socket = io(import.meta.env.VITE_WS_URL ?? '/ws', {
      transports: ['websocket'],
      reconnection: true,
      reconnectionAttempts: 5
    });
    socketRef.current = socket;
    socket.on('agent-message', (payload: ChatMessage) => {
      setMessages((current) => [...current, payload]);
    });
    socket.on('connect', () => {
      console.debug('Agent chat connected');
    });
    socket.on('disconnect', () => {
      console.debug('Agent chat disconnected');
    });
    return () => {
      socket.disconnect();
    };
  }, []);

  useEffect(() => {
    if (viewportRef.current) {
      viewportRef.current.scrollTop = viewportRef.current.scrollHeight;
    }
  }, [messages]);

  const handleSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!input.trim()) {
      return;
    }

    const userMessage: ChatMessage = {
      id: crypto.randomUUID(),
      role: 'user',
      content: input,
      agent,
      createdAt: new Date().toISOString()
    };
    setMessages((current) => [...current, userMessage]);
    setInput('');
    setIsStreaming(true);

    try {
      const response = await rpc.call<{ task_id: string }>('agent.dispatch', {
        agent,
        prompt: userMessage.content,
        metadata: {
          source: 'studio-ui',
          request_id: userMessage.id
        }
      });
      setMessages((current) => [
        ...current,
        {
          id: response.task_id,
          role: 'system',
          content: `Dispatched task ${response.task_id}`,
          createdAt: new Date().toISOString(),
          agent
        }
      ]);
    } catch (error) {
      setMessages((current) => [
        ...current,
        {
          id: crypto.randomUUID(),
          role: 'system',
          content: error instanceof Error ? error.message : 'Failed to dispatch agent task',
          createdAt: new Date().toISOString(),
          agent
        }
      ]);
    } finally {
      setIsStreaming(false);
    }
  };

  return (
    <div className="flex h-full flex-col bg-[color:var(--bg-primary)]/80">
      <div className="flex items-center justify-between border-b border-slate-800/60 bg-[color:var(--panel)] px-4 py-2">
        <div className="flex items-center space-x-2 text-sm">
          <label htmlFor="agent" className="text-[color:var(--text-secondary)]">
            Agent
          </label>
          <select
            id="agent"
            value={agent}
            onChange={(event) => setAgent(event.target.value)}
            className="rounded-md border border-[color:var(--accent-1)]/40 bg-transparent px-3 py-1 text-[color:var(--text-primary)] focus:outline-none"
          >
            {agentOptions.map((option) => (
              <option key={option.id} value={option.id} className="text-slate-900">
                {option.label}
              </option>
            ))}
          </select>
        </div>
        <span className="text-xs uppercase tracking-wide text-[color:var(--text-secondary)]">
          {isStreaming ? 'Awaiting agent response…' : 'Idle'}
        </span>
      </div>
      <div ref={viewportRef} className="flex-1 space-y-3 overflow-y-auto bg-[color:var(--bg-primary)]/70 p-4">
        {messages.map((message) => (
          <div
            key={message.id}
            className={`rounded-lg border border-slate-700/50 bg-[color:var(--panel)] p-3 text-sm shadow-panel ${
              message.role === 'user' ? 'border-[color:var(--accent-1)]/50' : ''
            }`}
          >
            <div className="mb-1 flex items-center justify-between text-xs text-[color:var(--text-secondary)]">
              <span>
                {message.role.toUpperCase()} {message.agent ? `· ${message.agent}` : ''}
              </span>
              <span>{new Date(message.createdAt).toLocaleTimeString()}</span>
            </div>
            <p className="whitespace-pre-wrap text-[color:var(--text-primary)]">{message.content}</p>
          </div>
        ))}
      </div>
      <form onSubmit={handleSubmit} className="border-t border-slate-800/60 bg-[color:var(--panel)] p-4">
        <div className="flex space-x-3">
          <textarea
            value={input}
            onChange={(event) => setInput(event.target.value)}
            placeholder="Describe the task for the selected agent…"
            className="h-24 flex-1 resize-none rounded-md border border-transparent bg-[color:var(--bg-secondary)]/70 px-3 py-2 text-sm text-[color:var(--text-primary)] focus:border-[color:var(--accent-1)]/60 focus:outline-none"
          />
          <button
            type="submit"
            disabled={isStreaming}
            className="btn-primary h-24 rounded-md px-4 text-sm font-semibold uppercase tracking-wide"
          >
            {isStreaming ? 'Streaming…' : 'Send'}
          </button>
        </div>
      </form>
    </div>
  );
}
