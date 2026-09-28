ALTER TABLE roles ADD COLUMN runtime_options_json TEXT;
ALTER TABLE sessions ADD COLUMN runtime_options_json TEXT;
ALTER TABLE slots ADD COLUMN runtime_options_json TEXT;
