PRAGMA legacy_alter_table = OFF;

ALTER TABLE runners RENAME TO roles;
ALTER TABLE slots RENAME COLUMN runner_id TO role_id;
ALTER TABLE sessions RENAME COLUMN runner_id TO role_id;
