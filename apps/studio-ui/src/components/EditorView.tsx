import { useEffect, useMemo, useState } from 'react';
import Editor, { OnChange } from '@monaco-editor/react';
import { useStudioContext } from '../hooks/useStudioContext';

interface ProjectFile {
  path: string;
  content: string;
  language: string;
}

const languageByExtension: Record<string, string> = {
  rs: 'rust',
  ts: 'typescript',
  tsx: 'typescript',
  js: 'javascript',
  json: 'json',
  toml: 'toml',
  sql: 'sql',
  md: 'markdown'
};

function inferLanguage(path: string) {
  const extension = path.split('.').pop();
  if (!extension) {
    return 'plaintext';
  }
  return languageByExtension[extension] ?? 'plaintext';
}

export function EditorView() {
  const { rpc, refreshTokenUsage } = useStudioContext();
  const [files, setFiles] = useState<ProjectFile[]>([]);
  const [activeFile, setActiveFile] = useState<ProjectFile | null>(null);
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const loadInitial = async () => {
      try {
        const project = await rpc.call<{ project_id: string; files: ProjectFile[] }>('project.open', {
          project_id: 'active'
        });
        setFiles(project.files);
        setActiveFile(project.files[0] ?? null);
      } catch (err) {
        console.warn('Falling back to empty project', err);
        const fallback: ProjectFile = {
          path: 'README.md',
          content: '# Welcome to CyberDevStudio\n',
          language: 'markdown'
        };
        setFiles([fallback]);
        setActiveFile(fallback);
      }
    };
    loadInitial().catch((err) => console.error('Failed to load project', err));
  }, [rpc]);

  useEffect(() => {
    refreshTokenUsage().catch((err) => console.warn('Token refresh failed', err));
  }, [refreshTokenUsage]);

  const onEditorChange: OnChange = (value) => {
    if (!activeFile) {
      return;
    }
    setActiveFile({ ...activeFile, content: value ?? '' });
    setFiles((current) =>
      current.map((file) => (file.path === activeFile.path ? { ...file, content: value ?? '' } : file))
    );
  };

  const handleSave = async () => {
    if (!activeFile) {
      return;
    }
    setIsSaving(true);
    setError(null);
    try {
      await rpc.call('project.file.save', {
        project_id: 'active',
        path: activeFile.path,
        content: btoa(unescape(encodeURIComponent(activeFile.content)))
      });
    } catch (err) {
      if (err instanceof Error) {
        setError(err.message);
      } else {
        setError('Unable to save file');
      }
    } finally {
      setIsSaving(false);
    }
  };

  const language = useMemo(() => {
    if (!activeFile) {
      return 'plaintext';
    }
    return activeFile.language || inferLanguage(activeFile.path);
  }, [activeFile]);

  return (
    <div className="flex h-full flex-col bg-[color:var(--bg-primary)]/80">
      <div className="flex items-center justify-between border-b border-slate-800/60 bg-[color:var(--panel)] px-4 py-2">
        <div className="flex items-center space-x-2">
          {files.map((file) => (
            <button
              key={file.path}
              onClick={() => setActiveFile(file)}
              className={`rounded-md px-3 py-1 text-sm transition ${
                activeFile?.path === file.path
                  ? 'bg-[color:var(--accent-1)]/20 text-[color:var(--accent-1)]'
                  : 'text-[color:var(--text-secondary)] hover:bg-[color:var(--accent-1)]/10'
              }`}
            >
              {file.path}
            </button>
          ))}
        </div>
        <div className="flex items-center space-x-3 text-xs text-[color:var(--text-secondary)]">
          {error && <span className="text-red-400">{error}</span>}
          <button
            onClick={handleSave}
            disabled={isSaving}
            className="btn-primary rounded-md px-4 py-2 text-xs font-semibold uppercase tracking-wide"
          >
            {isSaving ? 'Saving…' : 'Save'}
          </button>
        </div>
      </div>
      <div className="flex-1 bg-[color:var(--bg-primary)]/60">
        <Editor
          key={activeFile?.path ?? 'empty'}
          defaultLanguage={language}
          defaultValue={activeFile?.content ?? ''}
          theme="vs-dark"
          path={activeFile?.path}
          onChange={onEditorChange}
          options={{
            minimap: { enabled: false },
            fontSize: 14,
            tabSize: 2,
            fontFamily: 'JetBrains Mono',
            automaticLayout: true
          }}
        />
      </div>
    </div>
  );
}
