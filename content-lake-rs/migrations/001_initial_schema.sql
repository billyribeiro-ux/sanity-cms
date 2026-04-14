-- Phase 0: Core tables for the Content Lake

-- Projects (multi-tenant isolation)
CREATE TABLE IF NOT EXISTS projects (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Datasets (each project has multiple datasets)
CREATE TABLE IF NOT EXISTS datasets (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(project_id, name)
);

-- Documents (the core entity)
CREATE TABLE IF NOT EXISTS documents (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    dataset_id UUID NOT NULL REFERENCES datasets(id) ON DELETE CASCADE,
    document_id TEXT NOT NULL,
    doc_type TEXT NOT NULL,
    revision TEXT NOT NULL,
    content JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted BOOLEAN NOT NULL DEFAULT false,
    UNIQUE(dataset_id, document_id)
);

CREATE INDEX IF NOT EXISTS idx_documents_type ON documents(dataset_id, doc_type);
CREATE INDEX IF NOT EXISTS idx_documents_content ON documents USING GIN(content jsonb_path_ops);
CREATE INDEX IF NOT EXISTS idx_documents_updated ON documents(dataset_id, updated_at DESC);

-- Transaction log (event sourcing / history)
CREATE TABLE IF NOT EXISTS transactions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    dataset_id UUID NOT NULL REFERENCES datasets(id) ON DELETE CASCADE,
    transaction_id TEXT NOT NULL UNIQUE,
    author TEXT,
    mutations JSONB NOT NULL,
    effects JSONB,
    timestamp TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_transactions_dataset ON transactions(dataset_id, timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_transactions_tid ON transactions(transaction_id);

-- Transaction-Document junction (which docs a txn touched)
CREATE TABLE IF NOT EXISTS transaction_documents (
    transaction_id UUID NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    document_id TEXT NOT NULL,
    previous_rev TEXT,
    result_rev TEXT,
    PRIMARY KEY(transaction_id, document_id)
);
