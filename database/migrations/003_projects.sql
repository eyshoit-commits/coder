CREATE TABLE IF NOT EXISTS projects (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    description TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (user_id, name)
);

CREATE TABLE IF NOT EXISTS project_files (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    content BYTEA NOT NULL,
    sha256 BYTEA NOT NULL,
    size BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (project_id, path)
);

CREATE INDEX IF NOT EXISTS project_files_project_idx ON project_files(project_id);
CREATE INDEX IF NOT EXISTS project_files_path_idx ON project_files(path);

CREATE TABLE IF NOT EXISTS project_activity (
    id BIGSERIAL PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    user_id INTEGER REFERENCES users(id) ON DELETE SET NULL,
    action TEXT NOT NULL,
    detail JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS project_activity_project_idx ON project_activity(project_id);
CREATE INDEX IF NOT EXISTS project_activity_created_idx ON project_activity(created_at);

CREATE OR REPLACE FUNCTION update_project_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_projects_updated_at ON projects;
CREATE TRIGGER trg_projects_updated_at
BEFORE UPDATE ON projects
FOR EACH ROW
EXECUTE FUNCTION update_project_updated_at();

DROP TRIGGER IF EXISTS trg_project_files_updated_at ON project_files;
CREATE TRIGGER trg_project_files_updated_at
BEFORE UPDATE ON project_files
FOR EACH ROW
EXECUTE FUNCTION update_project_updated_at();

CREATE OR REPLACE FUNCTION touch_parent_project()
RETURNS TRIGGER AS $$
DECLARE
    target_id UUID;
BEGIN
    IF TG_OP = 'DELETE' THEN
        target_id := OLD.project_id;
    ELSE
        target_id := NEW.project_id;
    END IF;
    UPDATE projects SET updated_at = NOW() WHERE id = target_id;
    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    ELSE
        RETURN NEW;
    END IF;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_project_files_touch ON project_files;
CREATE TRIGGER trg_project_files_touch
AFTER INSERT OR UPDATE OR DELETE ON project_files
FOR EACH ROW
EXECUTE FUNCTION touch_parent_project();
