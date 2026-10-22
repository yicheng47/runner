ALTER TABLE roles
    ADD COLUMN avatar_style TEXT NOT NULL DEFAULT 'initials'
    CHECK (avatar_style IN ('initials', 'pixel'));
