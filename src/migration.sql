-- Weaver schema: project chat + issue tracking
-- All tables prefixed with weaver_ to avoid conflicts with host app

-- Chat tables
CREATE TABLE IF NOT EXISTS weaver_channels (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id  UUID NOT NULL,
    name        VARCHAR(100) NOT NULL,
    created_by  TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(project_id, name)
);
CREATE INDEX IF NOT EXISTS idx_weaver_channels_project ON weaver_channels(project_id);

CREATE TABLE IF NOT EXISTS weaver_messages (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    channel_id  UUID NOT NULL REFERENCES weaver_channels(id) ON DELETE CASCADE,
    user_id     TEXT NOT NULL,
    user_email  TEXT NOT NULL DEFAULT '',
    content     TEXT NOT NULL DEFAULT '',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_weaver_messages_channel ON weaver_messages(channel_id, created_at);

CREATE TABLE IF NOT EXISTS weaver_attachments (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    message_id  UUID REFERENCES weaver_messages(id) ON DELETE CASCADE,
    storage_key TEXT NOT NULL,
    url         TEXT NOT NULL,
    filename    TEXT NOT NULL,
    file_type   TEXT NOT NULL,
    file_size   INTEGER NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_weaver_attachments_message ON weaver_attachments(message_id);

-- Issues tables
CREATE TABLE IF NOT EXISTS weaver_labels (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id  UUID NOT NULL,
    name        VARCHAR(50) NOT NULL,
    color       VARCHAR(7) NOT NULL DEFAULT '#6B7280',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(project_id, name)
);
CREATE INDEX IF NOT EXISTS idx_weaver_labels_project ON weaver_labels(project_id);

CREATE TABLE IF NOT EXISTS weaver_issues (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id  UUID NOT NULL,
    number      INTEGER NOT NULL,
    title       VARCHAR(300) NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    status      VARCHAR(30) NOT NULL DEFAULT 'backlog',
    priority    VARCHAR(20) NOT NULL DEFAULT 'medium',
    assignee_id TEXT,
    created_by  TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(project_id, number)
);
CREATE INDEX IF NOT EXISTS idx_weaver_issues_project_status ON weaver_issues(project_id, status);

CREATE TABLE IF NOT EXISTS weaver_issue_labels (
    issue_id    UUID NOT NULL REFERENCES weaver_issues(id) ON DELETE CASCADE,
    label_id    UUID NOT NULL REFERENCES weaver_labels(id) ON DELETE CASCADE,
    PRIMARY KEY (issue_id, label_id)
);

CREATE TABLE IF NOT EXISTS weaver_comments (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    issue_id    UUID NOT NULL REFERENCES weaver_issues(id) ON DELETE CASCADE,
    user_id     TEXT NOT NULL,
    user_email  TEXT NOT NULL DEFAULT '',
    content     TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_weaver_comments_issue ON weaver_comments(issue_id, created_at);

-- Message edit/delete support
ALTER TABLE weaver_messages ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ;
ALTER TABLE weaver_messages ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ;
