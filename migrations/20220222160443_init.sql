-- Add migration script here
CREATE TABLE task_groups(id VARCHAR(32) PRIMARY KEY, created_at TIMESTAMPTZ NOT NULL, repo_org VARCHAR(255) NOT NULL, repo_name VARCHAR(255) NOT NULL, git_ref VARCHAR(255) NOT NULL, source TEXT NOT NULL);
CREATE TABLE tasks(id VARCHAR(32) PRIMARY KEY, name VARCHAR(255), status VARCHAR(255) NOT NULL, task_group VARCHAR(32) NOT NULL REFERENCES task_groups);

CREATE TYPE task AS (
  id VARCHAR(32),
  status VARCHAR(255),
  name VARCHAR(255)
);

