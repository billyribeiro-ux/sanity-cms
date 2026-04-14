-- Phase 2: user accounts for real JWT-based auth.

CREATE TABLE IF NOT EXISTS users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    role TEXT NOT NULL DEFAULT 'administrator',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
