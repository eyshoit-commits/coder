CREATE EXTENSION IF NOT EXISTS vector;
CREATE EXTENSION IF NOT EXISTS pgml;
CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE TABLE IF NOT EXISTS code_embeddings (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    project TEXT NOT NULL,
    path TEXT NOT NULL,
    content_hash CHAR(64) NOT NULL,
    embedding vector(768) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX IF NOT EXISTS code_embeddings_unique_idx
    ON code_embeddings(user_id, project, path, content_hash);

CREATE INDEX IF NOT EXISTS code_embeddings_vector_idx
    ON code_embeddings USING ivfflat (embedding vector_cosine_ops)
    WITH (lists = 100);

CREATE TABLE IF NOT EXISTS pgml_training_jobs (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    model_name TEXT NOT NULL,
    dataset JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    status TEXT NOT NULL DEFAULT 'pending',
    error TEXT
);

CREATE INDEX IF NOT EXISTS pgml_training_jobs_user_idx
    ON pgml_training_jobs(user_id, created_at DESC);

CREATE OR REPLACE FUNCTION record_code_embedding(
    p_user_id INTEGER,
    p_project TEXT,
    p_path TEXT,
    p_content TEXT,
    p_embedding vector
) RETURNS UUID AS $$
DECLARE
    v_hash CHAR(64);
    v_id UUID;
BEGIN
    v_hash := encode(digest(p_content, 'sha256'), 'hex');
    INSERT INTO code_embeddings (user_id, project, path, content_hash, embedding)
    VALUES (p_user_id, p_project, p_path, v_hash, p_embedding)
    ON CONFLICT (user_id, project, path, content_hash)
        DO UPDATE SET embedding = EXCLUDED.embedding, created_at = NOW()
    RETURNING id INTO v_id;
    RETURN v_id;
END;
$$ LANGUAGE plpgsql;
